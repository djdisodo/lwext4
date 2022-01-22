#![feature(try_blocks)]
#![feature(in_band_lifetimes)]
extern crate lwext4_sys as ffi;
extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use core::ffi::c_void;
use core::marker::PhantomData;
use core::mem::transmute;
use core::ops::{Deref, DerefMut};
use core::ptr::null_mut;
use core::slice::{from_raw_parts, from_raw_parts_mut};
use std::ffi::{CStr, CString};
use std::pin::Pin;
use ffi::*;
use num_traits::FromPrimitive;


macro_rules! wrap_field {
	($field: ident, $set_field: ident, $rt: ty) => {
		pub fn $field(&self) -> $rt {
			self.0.$field
		}

		pub fn $set_field(&mut self, $field: $rt) {
			self.0.$field = $field;
		}
	}
}

pub struct Ext4BlockDevice<T: Ext4BlockDeviceInterface>(ext4_blockdev, PhantomData<T>);

impl<T: Ext4BlockDeviceInterface> Ext4BlockDevice<T> {
	pub fn new(interface: T, config: Ext4BlockDeviceConfig) -> Pin<Box<Self>> {
		unsafe {
			let raw_interface = ext4_blockdev_iface {
				open: Some(<T as Ext4BlockDeviceInterfaceExt>::open),
				bread: Some(<T as Ext4BlockDeviceInterfaceExt>::bread),
				bwrite: Some(<T as Ext4BlockDeviceInterfaceExt>::bwrite),
				close: Some(<T as Ext4BlockDeviceInterfaceExt>::close),
				lock: Some(<T as Ext4BlockDeviceInterfaceExt>::lock),
				unlock: Some(<T as Ext4BlockDeviceInterfaceExt>::unlock),
				ph_bsize: config.block_size,
				ph_bcnt: config.block_count,
				ph_bbuf: vec![0u8; config.block_size as usize].leak().as_mut_ptr(),
				ph_refctr: 0,
				bread_ctr: 0,
				bwrite_ctr: 0,
				p_user: transmute(Box::leak(Box::new(interface)))
			};
			let device_raw = ext4_blockdev {
				bdif: Box::leak(Box::new(raw_interface)),
				part_offset: 0,
				part_size: config.part_size,
				bc: null_mut(),
				lg_bsize: 0,
				lg_bcnt: 0,
				cache_write_back: 0,
				fs: null_mut(),
				journal: null_mut()
			};
			Box::pin(Ext4BlockDevice(device_raw, Default::default()))
		}
	}

	pub fn register(self: Pin<&'a mut Ext4BlockDevice<T>>, mut name: String) -> Result<Ext4BlockDeviceRegisterHandle<'a, T>, Error> {
		unsafe {
			name.push('\0');
			let name = CString::from_vec_unchecked(name.into());
			errno_to_result(ext4_device_register(transmute(&self.0 as &ext4_blockdev), name.as_ptr()))?;
			Ok(Ext4BlockDeviceRegisterHandle(self, name))
		}
	}



	wrap_field!(part_offset, set_part_offset, u64);
	wrap_field!(part_size, set_part_size, u64);
	wrap_field!(lg_bsize, set_lg_bsize, u32);
	wrap_field!(lg_bcnt, set_lg_bcnt, u64);
	wrap_field!(cache_write_back, set_cache_write_back, u32);
}

impl<T: Ext4BlockDeviceInterface> Drop for Ext4BlockDevice<T> {
	fn drop(&mut self) {
		unsafe {
			let block_size = (*self.0.bdif).ph_bsize;
			Vec::<u8>::from_raw_parts((*self.0.bdif).ph_bbuf, 0, block_size as usize);
			Box::<T>::from_raw((*self.0.bdif).p_user as _);
			Box::<ext4_blockdev_iface>::from_raw(self.0.bdif as _);
		}
	}
}



impl<T: Ext4BlockDeviceInterface> Deref for Ext4BlockDevice<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		unsafe {
			transmute((*self.0.bdif).p_user)
		}
	}
}

impl<T: Ext4BlockDeviceInterface> DerefMut for Ext4BlockDevice<T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		unsafe {
			transmute((*self.0.bdif).p_user)
		}
	}
}

pub struct Ext4BlockDeviceRegisterHandle<'a, T: Ext4BlockDeviceInterface>(Pin<&'a mut Ext4BlockDevice<T>>, CString);

impl<T: Ext4BlockDeviceInterface> Ext4BlockDeviceRegisterHandle<'_, T> {
	pub fn mount(&mut self, mut mount_point: String, read_only: bool) -> Result<(), Error> {
		mount_point.push('\0');
		unsafe {
			errno_to_result(ext4_mount(self.1.as_ptr(), CStr::from_bytes_with_nul_unchecked(mount_point.as_bytes()).as_ptr(), read_only))
		}
	}
}


impl<T: Ext4BlockDeviceInterface> Drop for Ext4BlockDeviceRegisterHandle<'_, T> {
	fn drop(&mut self) {
		unsafe {
			ext4_device_unregister(self.1.as_ptr());
		}
	}
}

pub struct Ext4BlockDeviceConfig {
	block_size: u32,
	block_count: u64,
	part_size: u64,
}

impl Ext4BlockDeviceConfig {
}


pub trait Ext4BlockDeviceInterface: Sized {
	fn open(device: &mut Ext4BlockDevice<Self>) -> Result<(), Error> where Self: Sized;
	fn read_block(device: &mut Ext4BlockDevice<Self>, buf: &[u8], block_id: BlockId, block_count: u32) -> Result<(), Error>;
	fn write_block(device: &mut Ext4BlockDevice<Self>, buf: &[u8], block_id: BlockId, block_count: u32) -> Result<(), Error>;
	fn close(device: &mut Ext4BlockDevice<Self>) -> Result<(), Error>;
	fn lock(device: &mut Ext4BlockDevice<Self>) -> Result<(), Error>;
	fn unlock(device: &mut Ext4BlockDevice<Self>) -> Result<(), Error>;
}

trait Ext4BlockDeviceInterfaceExt {
	unsafe extern "C" fn open(bdev: *mut ext4_blockdev) -> errno_t;
	unsafe extern "C" fn bread(bdev: *mut ext4_blockdev, buf: *mut c_void, blk_id: u64, blk_cnt: u32) -> errno_t;
	unsafe extern "C" fn bwrite(bdev: *mut ext4_blockdev, buf: *const c_void, blk_id: u64, blk_cnt: u32) -> errno_t;
	unsafe extern "C" fn close(bdev: *mut ext4_blockdev) -> errno_t;
	unsafe extern "C" fn lock(bdev: *mut ext4_blockdev) -> errno_t;
	unsafe extern "C" fn unlock(bdev: *mut ext4_blockdev) -> errno_t;
}

impl<T: Ext4BlockDeviceInterface> Ext4BlockDeviceInterfaceExt for T {
	unsafe extern "C" fn open(bdev: *mut ext4_blockdev) -> errno_t {
		result_to_errno(try {
			let mut device: &mut Ext4BlockDevice<T> = transmute(bdev);
			T::open(&mut device)?;
		})
	}

	unsafe extern "C" fn bread(bdev: *mut ext4_blockdev, buf: *mut c_void, blk_id: u64, blk_cnt: u32) -> errno_t {
		result_to_errno(try {
			let mut device: &mut Ext4BlockDevice<T> = transmute(bdev);
			let bsize = (*device.0.bdif).ph_bsize;
			T::read_block(
				&mut device,
				from_raw_parts(transmute(buf),(blk_cnt * bsize) as usize),
				BlockId(blk_id),
				blk_cnt
			)?;
		})
	}

	unsafe extern "C" fn bwrite(bdev: *mut ext4_blockdev, buf: *const c_void, blk_id: u64, blk_cnt: u32) -> errno_t {
		result_to_errno(try {
			let mut device: &mut Ext4BlockDevice<T> = transmute(bdev);
			let bsize = (*device.0.bdif).ph_bsize;
			T::read_block(
				&mut device,
				from_raw_parts_mut(transmute(buf),(blk_cnt * bsize) as usize),
				BlockId(blk_id),
				blk_cnt
			)?;
		})
	}

	unsafe extern "C" fn close(bdev: *mut ext4_blockdev) -> errno_t {
		result_to_errno(try {
			let mut device: &mut Ext4BlockDevice<T> = transmute(bdev);
			T::close(&mut device)?;
		})
	}

	unsafe extern "C" fn lock(bdev: *mut ext4_blockdev) -> errno_t {
		result_to_errno(try {
			let mut device: &mut Ext4BlockDevice<T> = transmute(bdev);
			T::lock(&mut device)?;
		})
	}

	unsafe extern "C" fn unlock(bdev: *mut ext4_blockdev) -> errno_t {
		result_to_errno(try {
			let mut device: &mut Ext4BlockDevice<T> = transmute(bdev);
			T::unlock(&mut device)?;
		})
	}
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BlockId(u64);

/// from ext4_errno.h
/// TODO add error messages
#[derive(num_derive::FromPrimitive)]
#[repr(i32)]
pub enum Error {
	OperationNotPermitted = EPERM as i32,
	NoEntry = ENOENT as i32,
	Io = EIO as i32,
	NoDeviceOrAddress = ENXIO as i32, //????
	TooBig = E2BIG as i32,
	OutOfMemory = ENOMEM as i32,
	PermissionDenied = EACCES as i32,
	BadAddress = EFAULT as i32,
	FileExists = EEXIST as i32,
	NoDevice = ENODEV as i32,
	NotDirectory = ENOTDIR as i32,
	IsDirectory = EISDIR as i32,
	InvalidArgument = EINVAL as i32,
	FileTooBig = EFBIG as i32,
	NoSpace = ENOSPC as i32,
	ReadOnly = EROFS as i32,
	TooManyLinks = EMLINK as i32,
	Range = ERANGE as i32,
	DirNotEmpty = ENOTEMPTY as i32,
	NoData = ENODATA as i32,
	NotSupported = ENOTSUP as i32,
	InvalidError = 9999,
}

fn result_to_errno(result: Result<(), Error>) -> errno_t {
	(match result {
		Ok(()) => EOK,
		Err(e) => e as _
	}) as errno_t
}

//todo map error
fn errno_to_result(errno: errno_t) -> Result<(), Error> {
	if errno == (EOK as i32) {
		Ok(())
	} else {
		Err(Error::from_i32(errno).unwrap_or(Error::InvalidError))
	}
}