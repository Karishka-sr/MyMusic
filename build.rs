fn main() {
    // Стиль "material": виджеты Slint выглядят по Material Design,
    // светлая или тёмная тема берётся из настроек телефона.
    let config = slint_build::CompilerConfiguration::new().with_style("material".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("не удалось собрать ui/app.slint");
}
