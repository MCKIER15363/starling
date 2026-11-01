//! Touch input (horizon_client::touch): on the playing screen the touch controls, drawn over the
//! HUD and turned into the virtual pad, mouse motion and the hotbar, laid out as the player's
//! touch-layout.json says (edited in place: a long press on pause); on every other screen the
//! first finger is the pointer, with a close button over the screens a world opens. Android's
//! back key is Escape, and the chat raises the on-screen keyboard and moves above it. On the
//! screens with slots a finger works them as Bedrock's touch inventory does (slots.rs). The mods'
//! buttons are in the layout as the game's own (game/mod_widgets.rs).
mod slots;
mod text_mirror;

use super::{Game, Screen, mod_widgets::ModFace};
use horizon_client::debug_keys::KeyAction;
use horizon_client::gamepad::ServerStick;
use horizon_client::input_mode::InputMode;
use horizon_client::{
    Result,
    gamepad::Button as Pad,
    hud::HudBatch,
    touch::{
        self, Action, Button, Chip, EditorAction, Interact, Layout, LayoutConfig, LayoutEditor,
        Shape, Show, Target, TouchControls,
    },
    ui_theme::{Face, Ui, color},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, TouchPhase, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, KeyCode, NamedKey},
    window::WindowId,
};

pub(super) const HELP: &[&str] = &[
    "touch-layout [edit|done|reset]  the touch controls' layout (touch-layout.json): show it, edit it in place, save, or go back to the default",
];

/// How long a tap holds the use trigger: vanilla samples input once a tick.
const USE_PULSE: Duration = Duration::from_millis(60);
/// How long the phone buzzes when the player breaks a block, and when a held finger splits a
/// stack (milliseconds).
pub(super) const BREAK_BUZZ: u32 = 25;
const SPLIT_BUZZ: u32 = 15;
/// The layout's file, beside options.txt.
pub(super) const LAYOUT_FILE: &str = "touch-layout.json";

#[derive(Default)]
pub(super) struct Touch {
    controls: TouchControls,
    /// The finger acting as the mouse off the playing screen.
    pointer: touch::TouchPointer,
    /// The finger on a slot and the stack it picked (slots.rs).
    slots: slots::SlotTouch,
    /// A touch was seen (`has_touch`). The controls show while the input mode is the touch
    /// screen: from the start on Android, from this first touch elsewhere (game/input_mode.rs).
    seen: bool,
    /// The controls were live last frame (the playing screen).
    live: bool,
    use_until: Option<Instant>,
    /// A tap on a mob hits it: the attack trigger is held until then.
    attack_until: Option<Instant>,
    /// Where a tap landed, aimed along while its use or hit is held ([`USE_PULSE`]).
    tapped: Option<(Instant, [f32; 2])>,
    /// The press held on the view uses (a mob, the air) rather than breaks.
    hold_uses: bool,
    /// The gyroscope is listened to (the layout aims with it and the controls play).
    gyro_on: bool,
    /// The on-screen keyboard was asked for.
    keyboard: bool,
    /// The keyboard is wanted over the text field shown now: a typing screen opened, or a tap
    /// landed on the field; a tap beside it, the back key or the field going away end it.
    wanted: bool,
    /// The text field the keyboard was last decided for (`text_context`).
    context: Option<&'static str>,
    /// The keyboard has shown since it was asked for: its height falling to 0 then means the
    /// player put it away (the back key), which the game is not told otherwise.
    ime_seen: bool,
    /// The keyboard's editor on Android (text_mirror.rs).
    mirror: text_mirror::Mirror,
    /// The player's layout, read from touch-layout.json on the first frame.
    config: Option<LayoutConfig>,
    /// The layout editor, while it is open.
    editor: Option<LayoutEditor>,
    /// The chat bar's button last tapped and when (it lights up a moment).
    chat_key: Option<(usize, Instant)>,
    /// The finger on the chat bar: its moves and lift are the bar's.
    chat_finger: Option<u64>,
    /// Fingers on the mods' buttons: (finger, mod id, button id).
    mod_keys: Vec<(u64, String, String)>,
    /// The mods' buttons as last offered to the layout.
    faces: Vec<ModFace>,
    /// The Sneak button holds sneak now (`touch_sync_sneak`).
    sneaking: bool,
}

/// The chat bar's keys, left to right: history up and down, completion, the cursor; then Paste.
const CHAT_KEYS: [KeyCode; 5] = [
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::Tab,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
];

/// A pixel arrow centred on (cx, cy): a head 7 wide and a stem 3 wide. `dir`: 0 up, 1 down,
/// 2 left, 3 right.
fn arrow(
    batch: &mut HudBatch,
    atlas: &horizon_client::hud::HudAtlas,
    c: [f32; 2],
    dir: u8,
    ink: u32,
) {
    let [cx, cy] = c;
    for k in 0..4 {
        let (k, w) = (k as f32, 2.0 * k as f32 + 1.0);
        let [x, y, w, h] = match dir {
            0 => [cx - k, cy - 3.0 + k, w, 1.0],
            1 => [cx - k, cy + 3.0 - k, w, 1.0],
            2 => [cx - 3.0 + k, cy - k, 1.0, w],
            _ => [cx + 3.0 - k, cy - k, 1.0, w],
        };
        batch.fill(atlas, x, y, w, h, ink);
    }
    let [x, y] = match dir {
        0 => [cx - 1.0, cy + 1.0],
        1 => [cx - 1.0, cy - 3.0],
        2 => [cx + 1.0, cy - 1.0],
        _ => [cx - 3.0, cy - 1.0],
    };
    batch.fill(atlas, x, y, 3.0, 3.0, ink);
}

/// A pixel clipboard centred on (cx, cy), for Paste: a board 9 by 11, its clip and two lines.
fn clipboard(batch: &mut HudBatch, atlas: &horizon_client::hud::HudAtlas, c: [f32; 2], ink: u32) {
    let [cx, cy] = c;
    let (left, top) = (cx - 4.0, cy - 4.0);
    batch.fill(atlas, left, top, 1.0, 10.0, ink);
    batch.fill(atlas, left + 8.0, top, 1.0, 10.0, ink);
    batch.fill(atlas, left, top + 9.0, 9.0, 1.0, ink);
    batch.fill(atlas, left, top, 2.0, 1.0, ink);
    batch.fill(atlas, left + 7.0, top, 2.0, 1.0, ink);
    batch.fill(atlas, cx - 2.0, top - 1.0, 5.0, 2.0, ink);
    batch.fill(atlas, cx - 2.0, cy, 5.0, 1.0, ink);
    batch.fill(atlas, cx - 2.0, cy + 2.0, 5.0, 1.0, ink);
}

// The controls' looks, in window pixels: a plate, a pressed plate and a label at a control's
// opacity.
fn plate(a: f32) -> u32 {
    color::with_alpha(color::WHITE, 0.16 * a)
}
fn pressed(a: f32) -> u32 {
    color::with_alpha(color::WHITE, 0.38 * a)
}
fn ink(a: f32) -> u32 {
    color::with_alpha(color::WHITE, 0.85 * a)
}
fn circle(ui: &mut Ui, c: [f32; 2], r: f32, fill: u32) {
    let d = ui.density;
    ui.rounded(
        (c[0] - r) / d,
        (c[1] - r) / d,
        2.0 * r / d,
        2.0 * r / d,
        r / d,
        fill,
    );
}
fn label(ui: &mut Ui, c: [f32; 2], r: f32, text: &str, tint: u32) {
    let d = ui.density;
    let size = (r / d * 0.5).clamp(9.0, 16.0);
    ui.text_centered(
        Face::Semibold,
        size,
        c[0] / d,
        c[1] / d - size * 0.62,
        tint,
        text,
    );
}
/// A plate and an outline of any shape (circles as above).
fn plate_of(ui: &mut Ui, shape: Shape, c: [f32; 2], r: f32, fill: u32) {
    let d = ui.density;
    let [hw, hh] = shape.half(r);
    let corner = match shape {
        Shape::Square => r * 0.3,
        Shape::Round | Shape::Wide(_) => r,
    };
    ui.rounded(
        (c[0] - hw) / d,
        (c[1] - hh) / d,
        2.0 * hw / d,
        2.0 * hh / d,
        corner / d,
        fill,
    );
}
fn ring_of(ui: &mut Ui, shape: Shape, c: [f32; 2], r: f32, line: u32) {
    let d = ui.density;
    let [hw, hh] = shape.half(r);
    let corner = match shape {
        Shape::Square => r * 0.3,
        Shape::Round | Shape::Wide(_) => r,
    };
    ui.border(
        (c[0] - hw) / d,
        (c[1] - hh) / d,
        2.0 * hw / d,
        2.0 * hh / d,
        corner / d,
        line,
    );
}
/// A sprite over a button, `share` of its height, at an opacity.
fn icon(ui: &mut Ui, sprite: &str, c: [f32; 2], r: f32, share: f32, a: f32) {
    let gui = ui.batch.scale;
    let side = (2.0 * r * share / gui).round();
    let (x, y) = (
        (c[0] / gui - side / 2.0).round(),
        (c[1] / gui - side / 2.0).round(),
    );
    let tint = horizon_client::hud::rgba(255, 255, 255, (a.clamp(0.0, 1.0) * 255.0) as u8);
    horizon_client::ui::sprite_sized(ui.batch, ui.atlas, sprite, x, y, side, side, tint);
}
/// A label's size for a button's radius: a wide button's is its height's, not its width's.
fn text_r(shape: Shape, r: f32) -> f32 {
    match shape {
        Shape::Wide(_) => r * 1.5,
        Shape::Round | Shape::Square => r,
    }
}

/// The layout editor over whatever is behind it (the world, the title, the setup's settings):
/// every control where it will be, the selected one's chips, the opacity slider, and Done, Reset
/// and the movement control. `face` names a button or a mod's button (and gives the latter its
/// icon); `shape` is a button's; `skin` is a sprite drawn whole in a control's place (Starling's
/// look, packaging/touch, or a pack's).
pub(super) fn draw_editor(
    ui: &mut Ui,
    layout: &Layout,
    config: &LayoutConfig,
    editor: &LayoutEditor,
    face: &dyn Fn(Target) -> (String, Option<String>),
    shape: &dyn Fn(Button) -> Shape,
    skin: &dyn Fn(Target) -> Option<String>,
) {
    let d = ui.density;
    let (w, h) = (layout.width, layout.height);
    // The movement control as it moves the player now.
    let dpad = layout.dpad;
    ui.fill(0.0, 0.0, w / d, h / d, color::with_alpha(color::BG, 0.45));
    for (target, place, c, r) in LayoutEditor::targets(config, w, h) {
        // As it will look (at least faintly, to be found again), outlined if a ghost.
        let a = (place.opacity * config.opacity).max(0.35);
        let shape = match target {
            Target::Extra(i) => config.extra[i].shape,
            Target::Button(b) => shape(b),
            Target::Stick if dpad => Shape::Square,
            Target::Stick => Shape::Round,
        };
        // Drawn as it plays (a ghost only outlined, as it is not drawn while playing).
        if place.show != Show::Ghost
            && let Some(sprite) = skin(target)
        {
            let a = if place.show == Show::Off { 0.4 } else { a };
            icon(ui, &sprite, c, r, 1.0, a);
            if editor.selected == Some(target) {
                ring_of(ui, shape, c, r, color::ACCENT);
            }
            continue;
        }
        match place.show {
            Show::On => plate_of(ui, shape, c, r, pressed(a)),
            Show::Ghost => ring_of(ui, shape, c, r, ink(0.5)),
            Show::Off => plate_of(ui, shape, c, r, plate(0.5)),
        }
        if editor.selected == Some(target) {
            ring_of(ui, shape, c, r, color::ACCENT);
        }
        let (name, sprite) = match target {
            Target::Stick if dpad => ("D-pad".to_owned(), None),
            Target::Stick => ("Move".to_owned(), None),
            _ => face(target),
        };
        let tint = if place.show == Show::Off {
            ink(0.4)
        } else {
            ink(1.0)
        };
        // A mod's button with an icon shows it, as while playing (its name is on the opacity
        // slider once picked).
        let drawn = match target {
            Target::Extra(_) if !matches!(shape, Shape::Wide(_)) => sprite,
            _ => None,
        };
        match drawn {
            Some(sprite) => {
                let a = if place.show == Show::Off { 0.4 } else { 1.0 };
                icon(ui, &sprite, c, r, 0.6, a);
            }
            None => label(ui, c, text_r(shape, r), &name, tint),
        }
    }
    let shown = editor.selected.and_then(|t| {
        LayoutEditor::targets(config, w, h)
            .into_iter()
            .find(|(x, ..)| *x == t)
            .map(|(_, p, ..)| p.show)
    });
    for (chip, c, r) in editor.chips(config, w, h) {
        circle(ui, c, r, color::with_alpha(color::BG, 0.85));
        let text = match chip {
            Chip::Smaller => "-",
            Chip::Bigger => "+",
            Chip::Visibility => match shown {
                Some(Show::Ghost) => "Ghost",
                Some(Show::Off) => "Off",
                _ => "On",
            },
            Chip::Sneak => config.sneak.label(),
            Chip::Anchor if config.fixed_stick => "Fixed",
            Chip::Anchor => "Float",
        };
        label(ui, c, r, text, ink(1.0));
    }
    // The opacity slider: the selected control's, or everyone's.
    let ([x0, y], [x1, _]) = LayoutEditor::opacity_track(w, h);
    let value = editor.opacity(config);
    let u = layout.unit;
    ui.rounded(
        x0 / d,
        (y - 0.08 * u) / d,
        (x1 - x0) / d,
        0.16 * u / d,
        0.08 * u / d,
        color::with_alpha(color::BG, 0.85),
    );
    ui.rounded(
        x0 / d,
        (y - 0.08 * u) / d,
        (x1 - x0) * value / d,
        0.16 * u / d,
        0.08 * u / d,
        color::ACCENT,
    );
    circle(ui, [x0 + (x1 - x0) * value, y], 0.28 * u, ink(1.0));
    let whose = match editor.selected {
        Some(Target::Stick) => "Move".to_owned(),
        Some(Target::Button(b)) => b.label().to_owned(),
        Some(target @ Target::Extra(_)) => face(target).0,
        None => "all".to_owned(),
    };
    ui.text_centered(
        Face::Semibold,
        12.0,
        (x0 + x1) / 2.0 / d,
        (y - 0.75 * u) / d,
        ink(1.0),
        &format!("Opacity ({whose}): {:.0}%", value * 100.0),
    );
    for (action, c, r) in LayoutEditor::bar(w, h) {
        let fill = match action {
            EditorAction::Done => color::with_alpha(color::ACCENT, 0.9),
            EditorAction::Reset
            | EditorAction::Movement
            | EditorAction::Interact
            | EditorAction::Vibrate
            | EditorAction::Gyro => color::with_alpha(color::BG, 0.85),
        };
        circle(ui, c, r, fill);
        let text = match action {
            EditorAction::Done => "Done",
            EditorAction::Reset => "Reset",
            // The movement control picked (a press switches it): auto is the D-pad where the
            // stick moves as the keys.
            EditorAction::Movement => match config.dpad {
                Some(true) => "D-pad",
                Some(false) => "Stick",
                None => "Auto",
            },
            // What a tap and a press act on: what is touched, or the crosshair's.
            EditorAction::Interact => config.interact.label(),
            EditorAction::Vibrate if config.vibrate => "Vib on",
            EditorAction::Vibrate => "Vib off",
            EditorAction::Gyro if config.gyro => "Gyro on",
            EditorAction::Gyro => "Gyro off",
        };
        label(ui, c, r, text, ink(1.0));
    }
}

impl Game<'_> {
    /// The player plays with a touch screen: a touch was seen, and on Android no keyboard, mouse
    /// or controller took over since (game/input_mode.rs).
    pub(super) fn has_touch(&self) -> bool {
        self.touch.seen && self.touch_shown()
    }
    fn touch_playing(&self) -> bool {
        self.screen == Screen::Playing
            && self.title.is_none()
            && self.player.is_some()
            && !self.over_world_screen()
    }

    /// A screen drawn over the world without leaving the playing screen: the sign editor, a
    /// book, a command block, a dialog. Fingers are a pointer there, as on any other screen.
    fn over_world_screen(&self) -> bool {
        self.sign_edit.is_some()
            || self.book.is_some()
            || self.command_block_screen.is_some()
            || self.mods.screen.is_some()
            || self.server_packs.prompting()
            || self.dialog_open()
    }

    /// A screen opened in a world that the back key or the close button leaves.
    fn touch_closable(&self) -> bool {
        self.title.is_none()
            && (!matches!(self.screen, Screen::Playing | Screen::Dead) || self.over_world_screen())
    }

    /// The touch controls turn the view as a grabbed pointer does (mouse.rs).
    pub(super) fn touch_turns(&self) -> bool {
        self.touch.live && self.touch.editor.is_none()
    }

    pub(super) fn touch_size(&self) -> [u32; 2] {
        self.window.as_ref().map_or([1, 1], |w| {
            let s = w.inner_size();
            [s.width.max(1), s.height.max(1)]
        })
    }

    fn touch_layout(&self) -> Layout {
        let [w, h] = self.touch_size();
        // The scale the HUD is drawn at, which the GUI Scale option only caps.
        let scale = HudBatch::new([w, h]).scale;
        let mut layout = match &self.touch.config {
            Some(config) => Layout::from_config(config, w as f32, h as f32, scale),
            None => Layout::new(w as f32, h as f32, scale),
        };
        layout.dpad = self.touch_dpad();
        layout
    }

    /// Whether the D-pad moves the player: as the layout picked, else when the stick moves as the
    /// keys on this server (their eight directions on screen).
    fn touch_dpad(&self) -> bool {
        let picked = self.touch.config.as_ref().and_then(|c| c.dpad);
        picked.unwrap_or(self.remote_stick_mode() == Some(ServerStick::Keys))
    }

    /// Taps and presses on the view act on what is under the finger now: tap to interact, the
    /// touch controls in play, in first person (the ray goes through the finger from the eyes).
    pub(super) fn touch_taps(&self) -> bool {
        self.touch.live
            && self.touch.editor.is_none()
            && self.touch_shown()
            && self.camera_type.first_person()
            && self
                .touch
                .config
                .as_ref()
                .is_none_or(|c| c.interact == Interact::Tap)
    }

    /// The window pixel tap to interact aims through: a finger that may still tap or holds, or
    /// a tap whose click is still held.
    fn touch_aim_point(&self) -> Option<[f32; 2]> {
        if !self.touch_taps() {
            return None;
        }
        self.touch.controls.aim().or_else(|| {
            self.touch
                .tapped
                .filter(|(until, _)| Instant::now() < *until)
                .map(|(_, p)| p)
        })
    }

    /// The unit ray from the eyes through the finger on the screen (the camera's own projection:
    /// its vertical field of view and the window's shape), for [`Game::aim`].
    pub(super) fn touch_aim_dir(&self) -> Option<[f64; 3]> {
        let p = self.touch_aim_point()?;
        let fov = self.locator_camera?.3;
        let [w, h] = self.touch_size().map(f64::from);
        let (sx, sy) = (
            2.0 * f64::from(p[0]) / w - 1.0,
            1.0 - 2.0 * f64::from(p[1]) / h,
        );
        let t = (f64::from(fov).to_radians() / 2.0).tan();
        let forward = glam::DVec3::from_array(super::view_vector(self.yaw, self.pitch));
        let yaw = f64::from(self.yaw).to_radians();
        // The screen's right (yaw 0 faces +z, its right is -x) and up.
        let right = glam::DVec3::new(-yaw.cos(), 0.0, -yaw.sin());
        let up = right.cross(forward);
        let dir = forward + right * (sx * t * w / h) + up * (sy * t);
        Some(dir.normalize().to_array())
    }

    /// The look along [`Game::touch_aim_dir`] (yaw near the player's own): what the server is
    /// told the player looks at and places with while a finger aims, so a block or mob acted on
    /// off the crosshair is in view to it (and its anticheat).
    pub(super) fn touch_aim_rotation(&self) -> Option<(f32, f32)> {
        let [x, y, z] = self.touch_aim_dir()?;
        let yaw = (-x).atan2(z).to_degrees() as f32;
        let pitch = (-y).clamp(-1.0, 1.0).asin().to_degrees() as f32;
        let turn = (yaw - self.yaw).rem_euclid(360.0);
        let turn = if turn > 180.0 { turn - 360.0 } else { turn };
        Some((self.yaw + turn, pitch))
    }

    /// A block, with no mob before it, under the finger (or the crosshair), that a press breaks:
    /// a content mod's cran
