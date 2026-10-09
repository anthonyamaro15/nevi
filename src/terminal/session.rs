//! Taking over the terminal and handing it back to the shell: startup, exit,
//! external commands (`:!`, lazygit), and job-control suspend (Ctrl-Z).

use super::Terminal;
use crossterm::{
    cursor,
    event::{
        self, DisableBracketedPaste, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute, terminal,
};
use std::io;

/// What came of a request to suspend the editor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendOutcome {
    /// The process stopped and the shell continued it, or the stop was
    /// dropped and returned at once. Either way the screen is back.
    Resumed,
    /// Stopping is ignored here, as in a bash `$(...)`, or the platform has
    /// no job control, so the screen was never handed over.
    Unavailable,
}

impl Terminal {
    /// Put the editor screen up: raw mode, the alternate screen, focus
    /// reporting, and unambiguous key codes.
    pub(super) fn enter_editor_screen(&mut self) -> io::Result<()> {
        self.set_raw_mode(true)?;
        execute!(
            self.stdout,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            event::EnableFocusChange,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )
    }

    /// Give the terminal back the way the shell expects it. Mouse capture is
    /// switched off here and comes back with the next render.
    pub(super) fn leave_editor_screen(&mut self) -> io::Result<()> {
        let written = execute!(
            self.stdout,
            event::DisableMouseCapture,
            event::DisableFocusChange,
            DisableBracketedPaste,
            PopKeyboardEnhancementFlags,
            cursor::SetCursorStyle::DefaultUserShape,
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        self.mouse_capture_enabled = false;
        // Leave raw mode even if the write failed: a shell stuck in raw mode
        // is worse than a stray escape code.
        let cooked = self.set_raw_mode(false);
        written.and(cooked)
    }

    fn set_raw_mode(&self, enabled: bool) -> io::Result<()> {
        if !self.owns_tty {
            return Ok(());
        }
        if enabled {
            terminal::enable_raw_mode()
        } else {
            terminal::disable_raw_mode()
        }
    }

    /// Suspend like Vim's Ctrl-Z and `:stop`: hand the terminal to the shell,
    /// stop the process, and take the screen back once `fg` continues it.
    /// Raw mode turns off the tty's own Ctrl-Z handling, so the editor has to
    /// send the stop itself.
    pub fn suspend(&mut self) -> anyhow::Result<SuspendOutcome> {
        if !job_control::can_stop() {
            return Ok(SuspendOutcome::Unavailable);
        }
        self.suspend_with(job_control::stop)
    }

    /// `stop` returns once the process is continued. Nothing here waits for
    /// SIGCONT: where the stop is dropped, as in a zsh `$(...)`, it returns
    /// at once and the editor just redraws. Neovim gets stuck in that spot
    /// with the shell's screen showing.
    pub(super) fn suspend_with(
        &mut self,
        stop: impl FnOnce() -> io::Result<()>,
    ) -> anyhow::Result<SuspendOutcome> {
        if let Err(error) = self.leave_editor_screen() {
            self.enter_editor_screen()?;
            return Err(error.into());
        }
        let stopped = stop();
        self.enter_editor_screen()?;
        stopped?;
        Ok(SuspendOutcome::Resumed)
    }
}

#[cfg(unix)]
mod job_control {
    use std::io;

    /// False when SIGTSTP is ignored, which is how bash runs `$(...)`.
    /// Stopping would do nothing there, so the screen stays up.
    pub fn can_stop() -> bool {
        // SAFETY: sigaction is plain data, and a null new action only reads
        // the current disposition into it.
        let mut current: libc::sigaction = unsafe { std::mem::zeroed() };
        let read = unsafe { libc::sigaction(libc::SIGTSTP, std::ptr::null(), &mut current) };
        read != 0 || current.sa_sigaction != libc::SIG_IGN
    }

    /// Stop the whole process group, like Vim, so language servers started
    /// by the editor pause with it and `fg` continues them all.
    pub fn stop() -> io::Result<()> {
        // SAFETY: kill has no memory-safety preconditions.
        if unsafe { libc::kill(0, libc::SIGTSTP) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(not(unix))]
mod job_control {
    pub fn can_stop() -> bool {
        false
    }

    pub fn stop() -> std::io::Result<()> {
        Ok(())
    }
}
