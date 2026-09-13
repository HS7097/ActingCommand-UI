// SPDX-License-Identifier: AGPL-3.0-only
fn main() {
    // No style pin: the console draws itself from the std-widgets Palette, so it
    // follows the platform default style and the system light/dark setting.
    slint_build::compile("ui/app.slint").expect("compile ui/app.slint");

    // Windows only: stamp the same mark on the .exe so Explorer and the taskbar
    // show it. The window icon itself comes from Window.icon in app.slint.
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("assets/acui.ico")
        .compile()
        .expect("compile assets/acui.ico into the exe resources");
}
