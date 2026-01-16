use std::env;
use std::path::PathBuf;

use nevi::{Editor, Terminal, load_config};
use nevi::terminal::handle_key;

fn main() -> anyhow::Result<()> {
    // Load configuration
    let settings = load_config();

    // Initialize editor with settings
    let mut editor = Editor::new(settings);

    // Open file from command line argument if provided
    if let Some(path) = env::args().nth(1) {
        editor.open_file(PathBuf::from(path))?;
    }

    // Initialize terminal
    let mut terminal = Terminal::new()?;

    // Get initial size
    let (width, height) = Terminal::size()?;
    editor.set_size(width, height);

    // Main event loop
    loop {
        // Render
        terminal.render(&editor)?;

        // Check if we should quit
        if editor.should_quit {
            break;
        }

        // Handle pending external command (like lazygit)
        if let Some(cmd) = editor.pending_external_command.take() {
            if let Err(e) = terminal.run_external_process(&cmd) {
                editor.set_status(format!("Error running command: {}", e));
            }
            // Re-render after external process returns
            continue;
        }

        // Handle input
        let key = terminal.read_key()?;
        handle_key(&mut editor, key);

        // Update size (in case terminal was resized)
        if let Ok((w, h)) = Terminal::size() {
            editor.set_size(w, h);
        }
    }

    Ok(())
}
