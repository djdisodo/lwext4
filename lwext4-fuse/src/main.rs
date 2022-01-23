#![feature(seek_stream_len)]

use std::fs::OpenOptions;
use std::io::{Seek, Write};
use gpt::disk::LogicalBlockSize;
use gpt::mbr::ProtectiveMBR;
use lwext4::{BlockDeviceConfig, SimpleBlockDeviceInterface};


fn main() {
    let mut args = std::env::args();
    args.next();
    let path = args.next().unwrap();
    let mbr = ProtectiveMBR::from_disk(&mut std::fs::File::open(&path).unwrap(), LogicalBlockSize::Lb512).unwrap();
    let mut file = OpenOptions::new().read(true).write(true).open(path).unwrap();

    let partition = mbr.partition(0).unwrap();

    let mut config = BlockDeviceConfig::default();

    let bs: u64 = 512;
    config.block_size = bs as u32;
    config.part_size = bs * partition.lb_size as u64;
    config.part_offset = bs * partition.lb_start as u64;
    config.block_count = partition.lb_size as u64;


    /*
    let len = file.stream_len().unwrap();
    config.block_size = 512;
    config.part_size = len;
    config.part_offset = 0;
    config.block_count = len / 512;
    */
    let device = SimpleBlockDeviceInterface::new_device(file, config);
    let register_handle = device.register("a".to_string()).unwrap();
    let _mount_handle = register_handle.mount("/mp/", false).unwrap();
    let read_dir = lwext4::read_dir("/mp/").unwrap();
    let mut file = lwext4::File::create("/mp/write_test.txt").unwrap();
    lwext4::rename("/mp/mold", "/mp/mold_mv").unwrap();
    file.write_all("Hello, World!".as_bytes()).unwrap();
    for entry in read_dir {
        println!("{}", entry.path().to_str().unwrap());
        println!("is_dir: {}", entry.file_type().is_dir());
    }
}
