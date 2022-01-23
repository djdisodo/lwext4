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
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::ptr::null;
use ffi::*;
use num_traits::FromPrimitive;
use path_slash::PathExt;


pub struct BlockDevice<T: BlockDeviceInterface>(ext4_blockdev, PhantomData<T>);

impl<T: BlockDeviceInterface> BlockDevice<T> {
	pub fn new(interface: T) -> Pin<Box<BlockDevice<T>>> {
		unsafe {
			let raw_interface = ext4_blockdev_iface {
				open: Some(<T as BlockDeviceInterfaceExt>::open),
				bread: Some(<T as BlockDeviceInterfaceExt>::bread),
				bwrite: Some(<T as BlockDeviceInterfaceExt>::bwrite),
				close: Some(<T as BlockDeviceInterfaceExt>::close),
				lock: Some(<T as BlockDeviceInterfaceExt>::lock),
				unlock: Some(<T as BlockDeviceInterfaceExt>::unlock),
				ph_bsize: 0,
				ph_bcnt: 0,
				ph_bbuf: null_mut(),
				ph_refctr: 0,
				bread_ctr: 0,
				bwrite_ctr: 0,
				p_user: transmute(Box::leak(Box::new(interface)))
			};
			let device_raw = ext4_blockdev {
				bdif: Box::leak(Box::new(raw_interface)),
				part_offset: 0,
				part_size: 0,
				bc: null_mut(),
				lg_bsize: 0,
				lg_bcnt: 0,
				cache_write_back: 0,
				fs: null_mut(),
				journal: null_mut()
			};
			Box::pin(Self(device_raw, Default::default()))
		}
	}

	pub fn register(self: Pin<Box<Self>>, mut name: String) -> Result<BlockDeviceRegisterHandle<T>, Error> {
		unsafe {
			name.push('\0');
			let name = CString::from_vec_unchecked(name.into());
			errno_to_result(ext4_device_register(transmute(&self.0 as &ext4_blockdev), name.as_ptr()))?;
			Ok(BlockDeviceRegisterHandle(self, BlockDeviceRegisterName(name)))
		}
	}
}

impl<T: BlockDeviceInterface> Drop for BlockDevice<T> {
	fn drop(&mut self) {
		unsafe {
			let block_size = (*self.0.bdif).ph_bsize;
			let buf = (*self.0.bdif).ph_bbuf;
			if buf != null_mut() {
				Vec::<u8>::from_raw_parts(buf, 0, block_size as usize);
			}
			Box::<T>::from_raw((*self.0.bdif).p_user as _);
			Box::<ext4_blockdev_iface>::from_raw(self.0.bdif as _);
		}
	}
}



impl<T: BlockDeviceInterface> Deref for BlockDevice<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		unsafe {
			transmute((*self.0.bdif).p_user)
		}
	}
}

impl<T: BlockDeviceInterface> DerefMut for BlockDevice<T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		unsafe {
			transmute((*self.0.bdif).p_user)
		}
	}
}

struct BlockDeviceRegisterName(CString);

impl Drop for BlockDeviceRegisterName {
	fn drop(&mut self) {
		unsafe {
			ext4_device_unregister(self.0.as_ptr());
		}
	}
}

impl Deref for BlockDeviceRegisterName {
	type Target = CString;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

pub struct BlockDeviceRegisterHandle<T: BlockDeviceInterface>(Pin<Box<BlockDevice<T>>>, BlockDeviceRegisterName);

impl<T: BlockDeviceInterface> BlockDeviceRegisterHandle<T> {
	pub fn unregister(self) -> Pin<Box<BlockDevice<T>>> {
		self.0
	}
}

impl<T: BlockDeviceInterface> BlockDeviceRegisterHandle<T> {
	pub fn mount<P: AsRef<Path>>(&self, mount_point: P, read_only: bool) -> Result<BlockDeviceMountHandle<T>, Error> {
		unsafe {
			let cstring = to_cstring_dir(mount_point);
			errno_to_result(ext4_mount(self.1.as_ptr(), cstring.as_ptr(), read_only))?;
			Ok(BlockDeviceMountHandle(&self, cstring))
		}
	}
}


pub struct BlockDeviceMountHandle<'a, T: BlockDeviceInterface>(&'a BlockDeviceRegisterHandle<T>, CString);

impl<T: BlockDeviceInterface> BlockDeviceMountHandle<'_, T> {
	pub fn umount(&self) -> Result<(), Error> {
		unsafe {
			errno_to_result(ext4_umount(self.1.as_ptr()))
		}
	}
}

impl<T: BlockDeviceInterface> Drop for BlockDeviceMountHandle<'_, T> {
	fn drop(&mut self) {
		match self.umount() {
			_ => {}
		}
	}
}

pub type SimpleBlockDevice<T> = BlockDevice<SimpleBlockDeviceInterface<T>>;

pub struct SimpleBlockDeviceInterface<T: Read + Write + Seek>(T, BlockDeviceConfig);

impl<T: Read + Write + Seek> SimpleBlockDeviceInterface<T> {
	pub fn new_device(inner: T, config: BlockDeviceConfig) -> Pin<Box<BlockDevice<Self>>> {
		BlockDevice::new(Self(inner, config))
	}
}

impl<T: Read + Write + Seek> BlockDeviceInterface for SimpleBlockDeviceInterface<T> {
	fn open(&mut self) -> Result<BlockDeviceConfig, Error> where Self: Sized {
		Ok(self.1)
	}

	fn read_block(&mut self, mut buf: &mut [u8], block_id: u64, _block_count: u32) -> Result<(), Error> {
		self.0.seek(SeekFrom::Start(block_id * 512)).unwrap();
		self.0.read_exact(&mut buf).unwrap();
		Ok(())
	}

	fn write_block(&mut self, buf: &[u8], block_id: u64, _block_count: u32) -> Result<(), Error> {
		self.0.seek(SeekFrom::Start(block_id * 512)).unwrap();
		self.0.write_all(buf).unwrap();
		Ok(())
	}


	fn close(&mut self) -> Result<(), Error> {
		self.0.flush().unwrap();
		Ok(())
	}

	fn lock(&mut self) -> Result<(), Error> {
		Ok(())
	}

	fn unlock(&mut self) -> Result<(), Error> {
		Ok(())
	}
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BlockDeviceConfig {
	pub block_size: u32,
	pub block_count: u64,
	pub part_size: u64,
	pub part_offset: u64
}


pub trait BlockDeviceInterface: Sized {
	fn open(&mut self) -> Result<BlockDeviceConfig, Error> where Self: Sized;
	fn read_block(&mut self, buf: &mut [u8], block_id: u64, block_count: u32) -> Result<(), Error>;
	fn write_block(&mut self, buf: &[u8], block_id: u64, block_count: u32) -> Result<(), Error>;
	fn close(&mut self) -> Result<(), Error>;
	fn lock(&mut self) -> Result<(), Error>;
	fn unlock(&mut self) -> Result<(), Error>;
}

trait BlockDeviceInterfaceExt {
	unsafe extern "C" fn open(bdev: *mut ext4_blockdev) -> errno_t;
	unsafe extern "C" fn bread(bdev: *mut ext4_blockdev, buf: *mut c_void, blk_id: u64, blk_cnt: u32) -> errno_t;
	unsafe extern "C" fn bwrite(bdev: *mut ext4_blockdev, buf: *const c_void, blk_id: u64, blk_cnt: u32) -> errno_t;
	unsafe extern "C" fn close(bdev: *mut ext4_blockdev) -> errno_t;
	unsafe extern "C" fn lock(bdev: *mut ext4_blockdev) -> errno_t;
	unsafe extern "C" fn unlock(bdev: *mut ext4_blockdev) -> errno_t;
}

impl<T: BlockDeviceInterface> BlockDeviceInterfaceExt for T {
	unsafe extern "C" fn open(bdev: *mut ext4_blockdev) -> errno_t {
		result_to_errno(try {
			let mut device: &mut BlockDevice<T> = transmute(bdev);
			let config = T::open(&mut *device)?;
			(*device.0.bdif).ph_bsize = config.block_size;
			(*device.0.bdif).ph_bcnt = config.block_count;
			(*device.0.bdif).ph_bbuf = vec![0u8; config.block_size as usize].leak().as_mut_ptr();
			device.0.part_size = config.part_size;
			device.0.part_offset = config.part_offset;
		})
	}

	unsafe extern "C" fn bread(bdev: *mut ext4_blockdev, buf: *mut c_void, blk_id: u64, blk_cnt: u32) -> errno_t {
		result_to_errno(try {
			let device: &mut BlockDevice<T> = transmute(bdev);
			let bsize = (*device.0.bdif).ph_bsize;
			T::read_block(
				&mut *device,
				from_raw_parts_mut(transmute(buf),(blk_cnt * bsize) as usize),
				blk_id,
				blk_cnt
			)?;
		})
	}

	unsafe extern "C" fn bwrite(bdev: *mut ext4_blockdev, buf: *const c_void, blk_id: u64, blk_cnt: u32) -> errno_t {
		result_to_errno(try {
			let device: &mut BlockDevice<T> = transmute(bdev);
			let bsize = (*device.0.bdif).ph_bsize;
			T::write_block(
				&mut *device,
				from_raw_parts(transmute(buf),(blk_cnt * bsize) as usize),
				blk_id,
				blk_cnt
			)?;
		})
	}

	unsafe extern "C" fn close(bdev: *mut ext4_blockdev) -> errno_t {
		result_to_errno(try {
			let mut device: &mut BlockDevice<T> = transmute(bdev);
			T::close(&mut device)?;
		})
	}

	unsafe extern "C" fn lock(bdev: *mut ext4_blockdev) -> errno_t {
		result_to_errno(try {
			let device: &mut BlockDevice<T> = transmute(bdev);
			T::lock(&mut *device)?;
		})
	}

	unsafe extern "C" fn unlock(bdev: *mut ext4_blockdev) -> errno_t {
		result_to_errno(try {
			let device: &mut BlockDevice<T> = transmute(bdev);
			T::unlock(&mut *device)?;
		})
	}
}

pub struct ReadDir(ext4_dir, PathBuf);

pub fn read_dir<P: AsRef<Path>>(path: P) -> Result<ReadDir, Error> {
	let mut raw_dir = ext4_dir {
		f: ext4_file {
			mp: null_mut(),
			inode: 0,
			flags: 0,
			fsize: 0,
			fpos: 0
		},
		de: ext4_direntry {
			inode: 0,
			entry_length: 0,
			name_length: 0,
			inode_type: 0,
			name: [0u8; 255]
		},
		next_off: 0
	};
	unsafe {
		errno_to_result(ext4_dir_open(&mut raw_dir as _, to_cstring_dir(&path).as_ptr()))?;
		Ok(ReadDir(raw_dir, path.as_ref().to_path_buf()))
	}
}

impl ReadDir {
	pub fn rewind(&mut self) {
		unsafe {
			ext4_dir_entry_rewind(&mut self.0 as _)
		}
	}
}

impl ReadDir {
	pub fn as_file(&self) -> File {
		File(self.0.f.clone(), self.1.clone())
	}
}

impl Drop for ReadDir {
	fn drop(&mut self) {
		unsafe {
			ext4_dir_close(&mut self.0 as _);
		}
	}
}


impl Iterator for ReadDir {
	type Item = DirEntry;

	fn next(&mut self) -> Option<Self::Item> {
		unsafe {
			let result = ext4_dir_entry_next(&mut self.0 as _);
			if result == null() {
				None
			} else {
				Some(DirEntry((*transmute::<_, &ext4_direntry>(result)).clone(), self.1.to_path_buf()))
			}
		}
	}
}


pub struct DirEntry(ext4_direntry, PathBuf);

impl DirEntry {
	pub fn name(&self) -> &str {
		unsafe {
			CStr::from_bytes_with_nul_unchecked(&self.0.name).to_str().unwrap().trim_matches('\0')
		}
	}

	pub fn path(&self) -> PathBuf {
		self.1.join(self.name())
	}

	pub fn inode(&self) -> u32 {
		//isn't 64 bit inode supported?
		self.0.inode
	}

	pub fn file_type(&self) -> FileType {
		FileType(self.0.inode_type)
	}
}

pub struct Metadata {
	#[allow(dead_code)]
	inode: ext4_inode,
	file_type: FileType
}

impl Metadata {
	pub fn file_type(&self) -> Result<FileType, Error> {
		Ok(self.file_type)
	}
}

#[derive(Copy, Clone)]
pub struct FileType(u8);

impl FileType {
	pub fn is_dir(&self) -> bool {
		self.0 == 2
	}

	pub fn is_file(&self) -> bool {
		self.0 == 1
	}

	pub fn is_symlink(&self) -> bool {
		self.0 == 0 //TODO set proper value
	}
}

#[derive(Clone)]
pub struct File(ext4_file, PathBuf);

impl File {
	pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
		let mut raw_file = ext4_file {
			mp: null_mut(),
			inode: 0,
			flags: 0,
			fsize: 0,
			fpos: 0
		};
		unsafe {
			errno_to_result(ext4_fopen2(&mut raw_file, to_cstring(&path).as_ptr(), O_RDONLY as i32))?;
		}
		Ok(Self(raw_file, path.as_ref().to_path_buf()))
	}

	pub fn create<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
		let mut raw_file = ext4_file {
			mp: null_mut(),
			inode: 0,
			flags: 0,
			fsize: 0,
			fpos: 0
		};
		unsafe {
			errno_to_result(ext4_fopen2(&mut raw_file, to_cstring(&path).as_ptr(), (O_CREAT | O_RDWR) as i32))?;
		}
		Ok(Self(raw_file, path.as_ref().to_path_buf()))
	}
}

impl Drop for File {
	fn drop(&mut self) {
		unsafe {
			errno_to_result(ext4_fclose(&mut self.0)).unwrap();
		}
	}
}

impl Seek for File {
	fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
		let (origin, offset) = match pos {
			SeekFrom::Start(offset) => (SEEK_SET, offset as i64),
			SeekFrom::End(offset) => (SEEK_END, offset),
			SeekFrom::Current(offset) => (SEEK_CUR, offset)
		};
		unsafe {
			errno_to_result(ext4_fseek(&mut self.0 as _, offset, origin)).map_err(|x| x.into_std())?;
		}
		Ok(self.0.fpos)
	}
}

impl Read for File {
	fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
		unsafe {
			let mut read: size_t = 0;
			let buf_size = buf.len();
			errno_to_result(ext4_fread(&mut self.0, buf.as_mut_ptr() as _, buf_size as size_t, &mut read as _)).map_err(|x| x.into_std())?;
			Ok(read as usize)
		}
	}
}

impl Write for File {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		unsafe {
			let mut wrote: size_t = 0;
			let buf_size = buf.len();
			errno_to_result(ext4_fwrite(&mut self.0 as _, buf.as_ptr() as _, buf_size as size_t, &mut wrote as _)).map_err(|x| x.into_std())?;
			Ok(wrote as usize)
		}
	}

	fn flush(&mut self) -> std::io::Result<()> {
		unsafe {
			errno_to_result(ext4_cache_flush(to_cstring_dir(&self.1).as_ptr())).map_err(|x| x.into_std())?;
		}
		Ok(())
	}
}

pub fn copy<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<u64, Error> {
	let mut from = File::open(from)?;
	let mut to = File::create(to)?;
	std::io::copy(&mut from, &mut to).map_err(from_std)
}

pub fn create_dir<P: AsRef<Path>>(path: P) -> Result<(), Error> {
	unsafe {
		errno_to_result(ext4_dir_mk(to_cstring(path).as_ptr()))
	}
}

pub fn create_dir_all<P: AsRef<Path>>(path: P) -> Result<(), Error> {
	create_dir(path)
}

pub fn hard_link<P: AsRef<Path>, Q: AsRef<Path>>(original: P, link: Q) -> Result<(), Error> {
	unsafe {
		errno_to_result(ext4_flink(to_cstring(original).as_ptr(), to_cstring(link).as_ptr()))
	}
}

//pub fn read_link<P: AsRef<Path>>(path: P) -> Result<PathBuf, Error> {}

pub fn read_to_string<P: AsRef<Path>>(path: P) -> Result<String, Error> {
	let mut file = File::open(path)?;
	let mut string = String::new();
	file.read_to_string(&mut string).map_err(from_std)?;
	Ok(string)
}

pub fn remove_dir<P: AsRef<Path>>(path: P) -> Result<(), Error> {
	unsafe {
		errno_to_result(ext4_dir_rm(to_cstring(path).as_ptr()))
	}
}

pub fn remove_file<P: AsRef<Path>>(path: P) -> Result<(), Error> {
	unsafe {
		errno_to_result(ext4_fremove(to_cstring(path).as_ptr()))
	}
}

pub fn remove_dir_all<P: AsRef<Path>>(path: P) -> Result<(), Error> {
	for dir_en in read_dir(&path)? {
		let ty = dir_en.file_type();
		if ty.is_dir() {
			remove_dir_all(dir_en.path())?;
		} else {
			remove_file(dir_en.path())?;
		}
	}
	Ok(())
}

pub fn rename<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<(), Error> {
	unsafe {
		if let (Some(from_parent), Some(from_name)) = (from.as_ref().parent(), from.as_ref().file_name().map(|x| x.to_string_lossy())) {
			let trim = from_name.trim_matches('\0');
			if let Some(entry) = read_dir(from_parent)?.find(|x| x.name() == trim) {
				let from_cs = to_cstring(from);
				let to_cs = to_cstring(to);
				errno_to_result(if entry.file_type().is_dir() {
					ext4_dir_mv(from_cs.as_ptr(), to_cs.as_ptr())
				} else {
					ext4_frename(from_cs.as_ptr(), to_cs.as_ptr())
				})
			} else {
				Err(Error::InvalidArgument)
			}
		} else {
			Err(Error::InvalidArgument)
		}
	}
}


/// from ext4_errno.h
/// TODO add error messages
#[derive(num_derive::FromPrimitive, Debug)]
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

fn from_std(e: std::io::Error) -> Error {
	Error::from_std(e)
}

impl Error {
	fn from_std(e: std::io::Error) -> Error {
		errno_to_result(e.raw_os_error().unwrap()).unwrap_err()
	}

	fn into_std(self) -> std::io::Error {
		//idk
		std::io::Error::from_raw_os_error(self as i32)
	}
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

#[inline]
fn to_cstring<P: AsRef<Path>>(v: P) -> CString {
	let mut vec = v.as_ref().to_slash_lossy().as_bytes().to_vec();
	vec.push(0);
	unsafe {
		CString::from_vec_unchecked(vec)
	}
}

#[inline]
fn to_cstring_dir<P: AsRef<Path>>(v: P) -> CString {
	let mut vec = v.as_ref().to_slash_lossy().as_bytes().to_vec();
	vec.push('/' as u8);
	vec.push(0);
	unsafe {
		CString::from_vec_unchecked(vec)
	}
}

