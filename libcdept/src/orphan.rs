//! The programs CDePT waits for must not outlive it.
//!
//! A simulator started for a job kept running after CDePT quit, was killed or crashed
//! (found 2026-11-27: two dozen `ngspice -b` on a project that was already deleted).
//! Cancelling the queue on the way out cannot cover a crash or `kill -9`, so the kernel
//! does it: with the parent-death signal the child is killed as soon as its parent ends.
//!
//! The signal is tied to the **thread** that starts the child: use it only where that
//! thread also waits for the child. It reaches the direct child (the launch scripts of
//! Magic and netgen `exec` the program, so they count), not processes a program starts
//! itself. Programs meant to stay, such as the tool windows, are not started with it.

use std::process::Command;

/// Have the program of `command` killed when the calling thread, or CDePT, ends.
pub(crate) fn die_with_parent(command: &mut Command) -> &mut Command {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let parent = std::process::id();
        // SAFETY: between fork and exec only async-signal-safe calls are made (prctl,
        // getppid, _exit); nothing allocates or locks.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // The parent may have ended before the signal was asked for.
                if libc::getppid() as u32 != parent {
                    libc::_exit(127);
                }
                Ok(())
            });
        }
    }
    command
}
