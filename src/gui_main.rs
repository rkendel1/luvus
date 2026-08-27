//! Native desktop GUI entry point (behind `--features gui`).
//!
//! This is the entry point for the native desktop application built with
//! wgpu + winit. It provides the same functionality as the TUI but with
//! GPU-accelerated rendering in a native window.
//!
//! # Building
//!
//! ```bash
//! cargo build --release --features gui
//! ```
//!
//! # Current Status
//!
//! Phase 1: Stub implementation — the window opens but renders nothing yet.
//! Full feature parity with the TUI is planned for Phase 3.

// Re-declare the state_db module for the gui binary.
#[cfg(feature = "gui")]
mod state_db;

#[cfg(feature = "gui")]
fn main() -> anyhow::Result<()> {
    use anyhow::anyhow;

    // Initialize logging
    eprintln!("luvus-gui {} starting...", env!("CARGO_PKG_VERSION"));

    // Get home directory for state
    let home = dirs_home().ok_or_else(|| anyhow!("could not determine home directory"))?;
    eprintln!("Home directory: {}", home.display());

    // Initialize state database
    let _state_db = state_db::StateDb::new(&home)?;
    eprintln!("State database initialized");

    // Create and run the window
    run_window()?;

    Ok(())
}

#[cfg(feature = "gui")]
fn dirs_home() -> Option<std::path::PathBuf> {
    // Try common environment variables
    if let Some(home) = std::env::var_os("HOME") {
        return Some(std::path::PathBuf::from(home));
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return Some(std::path::PathBuf::from(home));
    }
    None
}

#[cfg(feature = "gui")]
fn run_window() -> anyhow::Result<()> {
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        window::{Window, WindowAttributes, WindowId},
    };

    struct App {
        window: Option<Window>,
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_none() {
                let attrs = WindowAttributes::default()
                    .with_title("Luvus")
                    .with_inner_size(winit::dpi::LogicalSize::new(1280, 720));

                match event_loop.create_window(attrs) {
                    Ok(window) => {
                        eprintln!("Window created successfully");
                        self.window = Some(window);
                    }
                    Err(e) => {
                        eprintln!("Failed to create window: {e}");
                    }
                }
            }
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _window_id: WindowId,
            event: WindowEvent,
        ) {
            match event {
                WindowEvent::CloseRequested => {
                    eprintln!("Window close requested");
                    event_loop.exit();
                }
                WindowEvent::RedrawRequested => {
                    // TODO: Render UI with wgpu
                    // For now, just request another redraw
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                _ => {}
            }
        }
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut app = App { window: None };
    event_loop.run_app(&mut app)?;

    Ok(())
}

#[cfg(not(feature = "gui"))]
fn main() {
    eprintln!("error: GUI not compiled; use `cargo build --features gui`");
    std::process::exit(1);
}
