// SPDX-License-Identifier: AGPL-3.0-only
fn main() {
    // No style pin: the console draws itself from the std-widgets Palette, so it
    // follows the platform default style and the system light/dark setting.
    slint_build::compile("ui/app.slint").expect("compile ui/app.slint");
}
