fn main() {
    // 编译 Slint UI 定义文件
    slint_build::compile("ui/main.slint").expect("Slint UI 编译失败");
}
