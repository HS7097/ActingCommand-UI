// SPDX-License-Identifier: GPL-3.0-only
fn main() {
    // The window imports the console's kit (crates/acui-app/ui/kit) where it lies; no copy.
    slint_build::compile("ui/wizard.slint").expect("compile ui/wizard.slint");

    // Windows only: the console's own mark on the .exe, so the two programs are one family in
    // Explorer and the taskbar. The window's icon comes from Window.icon in wizard.slint, out
    // of the same asset folder.
    #[cfg(windows)]
    winresource::WindowsResource::new()
        .set_icon("../acui-app/assets/acui.ico")
        .compile()
        .expect("compile ../acui-app/assets/acui.ico into the exe resources");
}
