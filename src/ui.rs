// SPDX-License-Identifier: GPL-3.0-or-later
//! The background, as a Wayland client.
//!
//! One **wlr-layer-shell background surface per output**, anchored to all four edges. Modeled on
//! `wlrix-desktop/src/ui/mod.rs`, which is the same kind of program one layer up, with three
//! deliberate differences:
//!
//! - **`Layer::Background`, not `Bottom`.** The bottom layer belongs to the desktop icons, and
//!   `wlrix-desktop` sits there precisely so a wallpaper client can have this one -- two clients
//!   sharing a layer have no defined order between them, so restarting one would draw it over the
//!   other. This is the arrangement the desktop's own comment describes; this program is the
//!   other half of it.
//! - **`set_exclusive_zone(-1)`**, not `0`. A wallpaper covers the whole output, including
//!   underneath a panel's exclusive zone. Zero would shrink the surface when a panel appeared and
//!   leave an unpainted strip. This is the greeter's backdrop rule rather than the desktop's.
//! - **One surface per output.** `wlrix-desktop` picks a single monitor for its icons; a wallpaper
//!   goes on all of them, each with its own size, scale and resolved [`Wallpaper`].
//!
//! Two more things follow from a wallpaper being opaque. The buffer is `Xrgb8888` rather than
//! `Argb8888`, and the surface declares an opaque region covering all of itself -- which lets the
//! compositor skip drawing anything underneath, and is free to say.
//!
//! There is no seat, no pointer and no keyboard interactivity. Clicks belong to the desktop above.
//!
//! `calloop` drives everything: the Wayland connection and the `SIGHUP` reload ping are both
//! sources on one loop, so nothing polls and nothing is done from signal context.

use std::collections::HashMap;
use std::time::Duration;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::calloop::EventLoop,
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output::WlOutput, wl_shm, wl_surface::WlSurface},
};

use crate::config::{Config, Wallpaper};
use crate::decode::Pictures;

/// One monitor's background surface.
struct Screen {
    layer: LayerSurface,
    /// The connector name, e.g. `DP-1`. `None` when the compositor does not advertise one, which
    /// means this monitor can never match a `[[output]]` override and takes the top-level
    /// settings; see [`Config::resolve`].
    name: Option<String>,
    /// The surface size in *logical* pixels, as configured.
    width: u32,
    height: u32,
    /// The output's integer scale. The buffer is `width * scale` by `height * scale`.
    scale: i32,
    /// What this monitor shows, resolved from the config when it was last read.
    wallpaper: Wallpaper,
    dirty: bool,
}

/// The whole program's state.
pub struct Background {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    compositor: CompositorState,
    layer_shell: LayerShell,
    pool: SlotPool,

    config: Config,
    pictures: Pictures,
    screens: HashMap<u32, Screen>,

    exit: bool,
}

/// Run the background until the compositor goes away.
pub fn run() -> Result<(), String> {
    let config = Config::load();

    let conn = Connection::connect_to_env()
        .map_err(|err| format!("no Wayland compositor to connect to: {err}"))?;
    let (globals, event_queue) =
        registry_queue_init(&conn).map_err(|err| format!("could not read the registry: {err}"))?;
    let qh = event_queue.handle();

    let mut event_loop: EventLoop<Background> =
        EventLoop::try_new().map_err(|err| format!("could not create the event loop: {err}"))?;
    let loop_handle = event_loop.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|err| format!("wl_compositor unavailable: {err}"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|err| format!("wlr-layer-shell unavailable: {err}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|err| format!("wl_shm unavailable: {err}"))?;
    // Grown on demand when the real output sizes arrive; this is only a starting guess.
    let pool = SlotPool::new(1920 * 1080 * 4, &shm)
        .map_err(|err| format!("could not create a buffer pool: {err}"))?;

    let mut background = Background {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        compositor,
        layer_shell,
        pool,
        config,
        // Registers the extra decoders. Once, here, before anything is opened.
        pictures: Pictures::new(),
        screens: HashMap::new(),
        exit: false,
    };

    // Kept for the loop below, to tell a compositor that has gone away from a real failure.
    let health = conn.clone();
    WaylandSource::new(conn, event_queue)
        .insert(loop_handle.clone())
        .map_err(|err| format!("could not drive Wayland from the loop: {err}"))?;

    // Re-read `background.toml` on `SIGHUP`, so a settings change applies without restarting.
    // `wlrix-settings-daemon` finds this process through the pidfile below; `kill -HUP` does the
    // same thing by hand.
    let (reload_ping, reload_source) = calloop::ping::make_ping()
        .map_err(|err| format!("could not create the reload ping: {err}"))?;
    loop_handle
        .insert_source(reload_source, |_, _, background: &mut Background| {
            background.reload_config();
        })
        .map_err(|err| format!("could not watch for reloads: {err}"))?;
    crate::signals::forward_reload_to_loop(reload_ping);

    // Held until `run` returns, then its guard removes the file -- so a live pidfile means a live
    // background.
    let _pidfile = crate::pidfile::write();

    // The surfaces need outputs, which arrive from the loop; one dispatch settles the output
    // list, and `new_output` covers anything that shows up later.
    let qh2 = qh.clone();
    event_loop
        .dispatch(Duration::from_millis(200), &mut background)
        .map_err(|err| format!("initial dispatch failed: {err}"))?;
    for output in background.output_state.outputs().collect::<Vec<_>>() {
        background.add_output(&qh2, output);
    }

    loop {
        if let Err(err) = event_loop.dispatch(Duration::from_secs(1), &mut background) {
            // The compositor going away is the ordinary end of a Wayland client's life, not a
            // failure: logging out looks exactly like this, and reporting it would put an error in
            // the session log every single time. A connection that is still usable means something
            // else broke, which is worth saying.
            return if health.flush().is_err() {
                Ok(())
            } else {
                Err(format!("event loop failed: {err}"))
            };
        }
        // Painted after each batch rather than on a frame callback: a wallpaper changes only when
        // something asks it to -- a configure, a monitor appearing, a reload -- and a frame
        // callback only arrives after a commit that asked for one, so waiting on it would stall
        // the moment nothing is moving.
        background.draw_dirty();
        if background.exit {
            return Ok(());
        }
    }
}

impl Background {
    /// Give `output` a background surface, if it does not have one.
    fn add_output(&mut self, qh: &QueueHandle<Self>, output: WlOutput) {
        let id = output.id().protocol_id();
        if self.screens.contains_key(&id) {
            return;
        }

        let info = self.output_state.info(&output);
        let name = info.as_ref().and_then(|info| info.name.clone());
        // Floored at 1: a compositor that reports 0 would otherwise give a zero-sized buffer.
        let scale = info.as_ref().map_or(1, |info| info.scale_factor).max(1);
        let wallpaper = self.config.resolve(name.as_deref());

        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Background,
            Some("wlrix-bg"),
            Some(&output),
        );
        layer.set_anchor(Anchor::all());
        // -1, not 0: a wallpaper covers the whole output, including underneath a panel's
        // exclusive zone. Zero would shrink the surface when a panel appeared and leave a strip of
        // nothing behind it.
        layer.set_exclusive_zone(-1);
        // Nothing here is clickable. The desktop one layer up owns the pointer.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        // (0, 0) with all four anchors: the compositor decides, and the configure says what.
        layer.set_size(0, 0);
        layer.commit();

        eprintln!(
            "wlrix-bg: {} at scale {scale}: {} ({})",
            name.as_deref().unwrap_or("an unnamed output"),
            wallpaper.image.as_ref().map_or_else(
                || "no picture".to_string(),
                |path| path.display().to_string()
            ),
            wallpaper.mode.name(),
        );

        self.screens.insert(
            id,
            Screen {
                layer,
                name,
                width: 0,
                height: 0,
                scale,
                wallpaper,
                dirty: false,
            },
        );
    }

    /// Re-read `background.toml` and apply what changed, on `SIGHUP`.
    ///
    /// What `wlrix-settings-daemon` sends after writing a setting, and what `kill -HUP` does by
    /// hand. A file that no longer parses falls back to built-in defaults rather than keeping the
    /// running config: `Config::load` reports the problem and returns the defaults, and a plain
    /// gray desktop is a visible, recoverable state -- unlike the desktop's icon layout, there is
    /// nothing here a user would lose by it.
    ///
    /// **Everything reloads live, including the per-output overrides.** Changing a wallpaper is a
    /// buffer swap, not a surface teardown, so there is no equivalent of the desktop's "`output`
    /// only takes effect on a restart".
    fn reload_config(&mut self) {
        self.config = Config::load();
        // The file may have been edited in place under an unchanged name, so a cached decode
        // cannot be trusted across a reload.
        self.pictures.clear();
        for screen in self.screens.values_mut() {
            screen.wallpaper = self.config.resolve(screen.name.as_deref());
            screen.dirty = true;
        }
        eprintln!("wlrix-bg: reloaded background.toml");
    }

    /// Paint every surface that needs it.
    fn draw_dirty(&mut self) {
        let ids: Vec<u32> = self
            .screens
            .iter()
            .filter(|(_, screen)| screen.dirty)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.draw(id);
        }
    }

    fn draw(&mut self, id: u32) {
        let Some(screen) = self.screens.get_mut(&id) else {
            return;
        };
        if screen.width == 0 || screen.height == 0 {
            return;
        }
        screen.dirty = false;

        // Physical pixels: the buffer is the logical size times the output's scale, and the
        // surface is told so with `set_buffer_scale`.
        let width = screen.width as i32 * screen.scale;
        let height = screen.height as i32 * screen.scale;
        let stride = width * 4;

        // Copied out so the pool and the decoder can be borrowed mutably below.
        let wallpaper = screen.wallpaper.clone();
        let layer = screen.layer.clone();
        let scale = screen.scale;

        let picture = wallpaper
            .image
            .as_deref()
            .and_then(|path| self.pictures.load(path));
        // Reborrowed immutably: `load` needs `&mut self.pictures`, and the buffer below needs
        // `&mut self.pool`.
        let picture = picture.cloned();

        let (buffer, pixels) =
            match self
                .pool
                .create_buffer(width, height, stride, wl_shm::Format::Xrgb8888)
            {
                Ok(pair) => pair,
                Err(err) => {
                    eprintln!("wlrix-bg: could not get a buffer to draw into: {err}");
                    return;
                }
            };

        crate::render::render(pixels, width, height, &wallpaper, picture.as_ref());

        let surface = layer.wl_surface();
        surface.set_buffer_scale(scale);
        // A wallpaper is opaque, and saying so lets the compositor skip everything underneath it.
        // Region coordinates are the surface's own -- logical, not buffer -- so this is the
        // pre-scale size.
        match Region::new(&self.compositor) {
            Ok(region) => {
                region.add(0, 0, width / scale, height / scale);
                surface.set_opaque_region(Some(region.wl_region()));
            }
            Err(err) => eprintln!("wlrix-bg: could not declare the surface opaque: {err}"),
        }
        surface.damage_buffer(0, 0, width, height);
        if let Err(err) = buffer.attach_to(surface) {
            eprintln!("wlrix-bg: could not attach the buffer: {err}");
            return;
        }
        surface.commit();
    }
}

impl CompositorHandler for Background {
    fn scale_factor_changed(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        surface: &WlSurface,
        new: i32,
    ) {
        // The buffer is allocated at the output's scale, so this has to be honored or a HiDPI
        // screen gets a wallpaper at a quarter of its resolution.
        let Some(screen) = self
            .screens
            .values_mut()
            .find(|screen| screen.layer.wl_surface() == surface)
        else {
            return;
        };
        let new = new.max(1);
        if screen.scale != new {
            screen.scale = new;
            screen.dirty = true;
        }
    }

    fn transform_changed(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new: wayland_client::protocol::wl_output::Transform,
    ) {
        // The compositor rotates the surface for us; the buffer's own contents do not change.
    }

    fn frame(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _surface: &WlSurface, _t: u32) {
        // Drawing is driven by the loop, not by frame callbacks; see `run`.
    }

    fn surface_enter(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }
}

impl OutputHandler for Background {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _c: &Connection, qh: &QueueHandle<Self>, output: WlOutput) {
        // A monitor plugged in mid-session gets a wallpaper without anything being restarted.
        self.add_output(qh, output);
    }

    fn update_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, output: WlOutput) {
        // The name can arrive after the output does -- `wl_output.name` is a separate event -- and
        // a `[[output]]` override cannot match until it has. So the wallpaper is resolved again
        // here rather than only at creation, and a change in either the name or the scale is a
        // repaint.
        let id = output.id().protocol_id();
        let info = self.output_state.info(&output);
        let name = info.as_ref().and_then(|info| info.name.clone());
        let scale = info.as_ref().map_or(1, |info| info.scale_factor).max(1);
        let wallpaper = self.config.resolve(name.as_deref());

        let Some(screen) = self.screens.get_mut(&id) else {
            return;
        };
        if screen.name != name || screen.scale != scale || screen.wallpaper != wallpaper {
            screen.name = name;
            screen.scale = scale;
            screen.wallpaper = wallpaper;
            screen.dirty = true;
        }
    }

    fn output_destroyed(&mut self, _c: &Connection, _q: &QueueHandle<Self>, output: WlOutput) {
        // Dropping the `Screen` destroys its layer surface. Nothing is moved anywhere: unlike the
        // desktop's icons, a wallpaper on a monitor that has gone has nowhere else to be.
        self.screens.remove(&output.id().protocol_id());
    }
}

impl LayerShellHandler for Background {
    fn closed(&mut self, _c: &Connection, _q: &QueueHandle<Self>, layer: &LayerSurface) {
        self.screens
            .retain(|_, screen| screen.layer.wl_surface() != layer.wl_surface());
        // Every surface gone and no outputs left means the compositor is going away; with outputs
        // still present it means this one surface was taken, which is not worth exiting over.
        if self.screens.is_empty() && self.output_state.outputs().next().is_none() {
            self.exit = true;
        }
    }

    fn configure(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(screen) = self
            .screens
            .values_mut()
            .find(|screen| screen.layer.wl_surface() == layer.wl_surface())
        else {
            return;
        };
        let (width, height) = configure.new_size;
        screen.width = width;
        screen.height = height;
        // Dirty even when the size is unchanged: a surface has no content until a buffer is
        // attached, so the first configure must still produce a paint.
        screen.dirty = true;
    }
}

impl ShmHandler for Background {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Background {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

delegate_compositor!(Background);
delegate_output!(Background);
delegate_shm!(Background);
delegate_layer!(Background);
delegate_registry!(Background);
