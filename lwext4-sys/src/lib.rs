#![allow(non_snake_case, non_camel_case_types, non_upper_case_globals)]
pub mod inode;
include!(concat!(env!("OUT_DIR"), "/ext4.rs"));