// SPDX-License-Identifier: GPL-3.0-only
fn main() {
    // No style pin, as in the console: the wizard draws itself from the
    // std-widgets Palette, so it follows the platform default style and the
    // system light/dark setting.
    slint_build::compile("ui/setup.slint").expect("compile ui/setup.slint");

    // Windows only: the console's own mark on the .exe, so the two programs
    // are one family in Explorer and the taskbar. The window icon comes from
    // Window.icon in setup.slint, out of the same asset directory.
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("../acui-app/assets/acui.ico")
        .compile()
        .expect("compile ../acui-app/assets/acui.ico into the exe resources");
}
