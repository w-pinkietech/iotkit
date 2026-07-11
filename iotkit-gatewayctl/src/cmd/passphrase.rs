use std::io;
use std::os::fd::RawFd;

use clap::Subcommand;
use rusqlite::Connection;

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Subcommand)]
pub enum PassphraseCommand {
    Reset,
}

pub fn run_passphrase_reset(conn: &Connection) -> AppResult<()> {
    let passphrase = read_confirmed_passphrase()?;
    let hash = iotkit_core_ops::hash_passphrase(&passphrase)?;
    drop(passphrase);
    iotkit_core_ops::reset_passphrase_with_hash(conn, &hash, "local_cli")?;
    println!("passphrase reset");
    Ok(())
}

fn read_confirmed_passphrase() -> AppResult<String> {
    let first = read_secret("new passphrase: ")?;
    if first.len() < 8 {
        return Err("passphrase must be at least 8 characters".into());
    }
    let second = read_secret("confirm passphrase: ")?;
    if first != second {
        return Err("passphrases do not match".into());
    }
    Ok(first)
}

fn read_secret(prompt: &str) -> io::Result<String> {
    eprint!("{prompt}");
    use std::io::Write;
    io::stderr().flush()?;
    let is_tty = unsafe { libc::isatty(libc::STDIN_FILENO) } == 1;
    let interrupt = is_tty.then(InterruptGuard::install).transpose()?;
    let _echo = is_tty.then(TerminalEchoGuard::hide).transpose()?;
    let mut line = String::new();
    let read_result = if is_tty {
        wait_for_line_or_interrupt(interrupt.as_ref().unwrap().read_fd)?;
        io::stdin().read_line(&mut line)
    } else {
        io::stdin().read_line(&mut line)
    };
    if is_tty {
        eprintln!();
    }
    if read_result? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "passphrase input ended",
        ));
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

struct TerminalEchoGuard {
    fd: RawFd,
    original: libc::termios,
}

impl TerminalEchoGuard {
    fn hide() -> io::Result<Self> {
        let fd = libc::STDIN_FILENO;
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let original = unsafe { original.assume_init() };
        let mut hidden = original;
        hidden.c_lflag &= !libc::ECHO;
        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &hidden) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd, original })
    }
}

impl Drop for TerminalEchoGuard {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.original) };
    }
}

struct InterruptGuard {
    read_fd: RawFd,
    old_mask: libc::sigset_t,
}

impl InterruptGuard {
    fn install() -> io::Result<Self> {
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe {
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGINT);
            libc::sigaddset(&mut set, libc::SIGTERM);
        }
        let mut old_mask = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        if unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, old_mask.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let read_fd = unsafe { libc::signalfd(-1, &set, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK) };
        if read_fd < 0 {
            unsafe {
                libc::pthread_sigmask(libc::SIG_SETMASK, old_mask.as_ptr(), std::ptr::null_mut())
            };
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            read_fd,
            old_mask: unsafe { old_mask.assume_init() },
        })
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.read_fd);
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.old_mask, std::ptr::null_mut());
        }
    }
}

fn wait_for_line_or_interrupt(signal_fd: RawFd) -> io::Result<()> {
    let mut fds = [
        libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: signal_fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    loop {
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if fds[1].revents & libc::POLLIN != 0 {
            let mut info = std::mem::MaybeUninit::<libc::signalfd_siginfo>::uninit();
            unsafe {
                libc::read(
                    signal_fd,
                    info.as_mut_ptr().cast(),
                    std::mem::size_of::<libc::signalfd_siginfo>(),
                );
            }
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "passphrase input interrupted",
            ));
        }
        if result > 0 && fds[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            return Ok(());
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}
