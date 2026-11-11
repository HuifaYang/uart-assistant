fn main() {
    println!("cargo:rerun-if-changed=ui");
    slint_build::compile("ui/app.slint").expect("failed to compile Slint UI");
}
