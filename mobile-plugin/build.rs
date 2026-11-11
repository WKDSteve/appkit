// The plugin has no command of its own for the page: the shell (Rust) calls its Android side, and the page reaches
// none of it. So the list of commands is empty and no permission of the page opens anything here.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .build();
}
