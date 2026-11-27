use nyaterm_native_protocol::x11::{self, Command, Event, Failure, X11_PROTOCOL_VERSION};

#[cfg(windows)]
mod private_file;
#[cfg(windows)]
mod process;
#[cfg(windows)]
mod server;

fn run() -> std::io::Result<()> {
    let command = x11::read::<Command>(&mut std::io::stdin().lock())?;
    let Command::Start { version, clipboard } = command else {
        return Ok(());
    };
    if version != X11_PROTOCOL_VERSION {
        return x11::write(
            &mut std::io::stdout().lock(),
            &Event::Failed {
                kind: Failure::IncompatibleProtocol,
            },
        );
    }
    #[cfg(windows)]
    {
        server::run(clipboard)
    }
    #[cfg(not(windows))]
    {
        let _ = clipboard;
        x11::write(
            &mut std::io::stdout().lock(),
            &Event::Failed {
                kind: Failure::UnsupportedPlatform,
            },
        )
    }
}

fn main() {
    if !matches!(std::panic::catch_unwind(run), Ok(Ok(()))) {
        let _ = x11::write(
            &mut std::io::stdout().lock(),
            &Event::Failed {
                kind: Failure::Startup,
            },
        );
        std::process::exit(1);
    }
}
