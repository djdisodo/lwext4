use std::{env, fs};
use std::path::PathBuf;
use fs_extra::dir::CopyOptions;
use make_cmd::make;

fn main() {
	println!("cargo:rerun-if-changed=./lwext4");
	let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
	fs_extra::dir::remove(&out_dir).unwrap();
	std::fs::create_dir_all(&out_dir).unwrap();
	let lwext4_dir = out_dir.join("lwext4");
	let mut copy_options = CopyOptions::new();
	copy_options.copy_inside = true;
	copy_options.overwrite = true;
	fs_extra::dir::copy("./lwext4", &out_dir, &copy_options).unwrap();
	make().current_dir(&lwext4_dir).arg("generic").status().unwrap();
	let build_generic_dir = lwext4_dir.join("build_generic");
	make().current_dir(&build_generic_dir).arg("lwext4").status().unwrap();
	println!("cargo:rustc-link-search={}", fs::canonicalize(build_generic_dir.join("src")).unwrap().to_str().unwrap());
	println!("cargo:rustc-link-lib=static=lwext4");
	fs_extra::dir::copy(lwext4_dir.join("include"), &build_generic_dir, &copy_options).unwrap();
	let bindings = bindgen::builder()
		.header(build_generic_dir.join("include").join("ext4.h").to_str().unwrap())
		.clang_arg(format!("-I{}", dbg!(build_generic_dir.join("include").to_str().unwrap())))
		.use_core()
		.parse_callbacks(Box::new(bindgen::CargoCallbacks))
		.generate().unwrap();
	bindings.write_to_file(out_dir.join("ext4.rs")).unwrap()
}