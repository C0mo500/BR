//! The pointer on the screen while files are dragged in from the file manager: winit reports no
//! motion during such a drag (and on X11 ignores the drop position), so the drop would land where
//! the pointer was last seen. On X11 the server says where it is; elsewhere FilmCraft's platform
//! layer does (Windows) or nothing can (the last position egui saw is used).

/// The pointer on the screen in physical pixels.
pub fn screen_position() -> Option<(i32, i32)> {
    #[cfg(target_os = "linux")]
    if let Some(p) = x11::pointer() {
        return Some(p);
    }
    filmcraft_platform::cursor::cursor_screen_position()
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::sync::{Mutex, OnceLock};

    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    use x11rb::rust_connection::RustConnection;

    /// One connection, opened the first time (none without an X display, e.g. on Wayland).
    fn connection() -> &'static Mutex<Option<(RustConnection, u32)>> {
        static CONN: OnceLock<Mutex<Option<(RustConnection, u32)>>> = OnceLock::new();
        CONN.get_or_init(|| {
            let conn = std::env::var_os("DISPLAY").and_then(|_| x11rb::connect(None).ok()).map(|(c, screen)| {
                let root = c.setup().roots.get(screen).map_or(0, |r| r.root);
                (c, root)
            });
            Mutex::new(conn)
        })
    }

    pub fn pointer() -> Option<(i32, i32)> {
        let guard = connection().lock().ok()?;
        let (conn, root) = guard.as_ref()?;
        let reply = conn.query_pointer(*root).ok()?.reply().ok()?;
        Some((i32::from(reply.root_x), i32::from(reply.root_y)))
    }
}
