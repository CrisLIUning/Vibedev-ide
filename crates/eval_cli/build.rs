fn main() {
    let cargo_toml =
        std::fs::read_to_string("../zed/Cargo.toml").expect("读取 crates/zed/Cargo.toml 失败");
    let version = cargo_toml
        .lines()
        .find(|line| line.starts_with("version = "))
        .expect("在 crates/zed/Cargo.toml 中未找到版本号")
        .split('=')
        .nth(1)
        .expect("无效的版本格式")
        .trim()
        .trim_matches('"');
    println!("cargo:rerun-if-changed=../zed/Cargo.toml");
    println!("cargo:rustc-env=ZED_PKG_VERSION={}", version);
}
