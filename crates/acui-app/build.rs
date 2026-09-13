// SPDX-License-Identifier: AGPL-3.0-only
fn main() {
    // The console draws its own light surfaces, so pin the widget style to the
    // light variant instead of following the system dark/light setting.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-light".to_string());
    slint_build::compile_with_config("ui/app.slint", config).expect("compile ui/app.slint");
}
