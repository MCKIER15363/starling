//! Touch controls for phones (horizon-android): a stick that walks (floating under the finger,
//! or fixed with its knob going to the finger), dragging anywhere else that turns the view (a tap there uses the held item, a still press breaks), buttons for
//! every other action (jump, sneak, attack, use, inventory, chat, commands, pause, camera, drop,
//! offhand, the debug screen), and the hotbar's slots to tap. Each finger is claimed by one
//! control when it lands and keeps it until it lifts.
//!
//! As Bedrock's control modes, the taps and presses act on what is under the finger ("tap to
//! interact", [`Interact::Tap`], the default: no crosshair, no Attack and Use buttons) or on what
//! the crosshair is on ("aim crosshair", [`Interact::Crosshair`]).
//!
//! Where the controls are is a [`LayoutConfig`] the player edits in place ([`LayoutEditor`]):
//! a long press on pause opens it. It is kept as JSON beside options.txt (touch-layout.json).
//! Mods add buttons of their own ([`Extra`]), placed and edited as the game's.
//!
//! This is the state machine only: positions are window pixels, and the game turns the
//! [`Action`]s into its virtual pad, mouse motion and hotbar (game/touch.rs). Off the playing
//! screen the first finger is the pointer ([`TouchPointer`]).
use serde_json::{Map, Value, json};
use std::time::{Duration, Instant};
use winit::{
    dpi::PhysicalPosition,
    event::{
        DeviceId, ElementState, MouseButton, MouseScrollDelta, Touch, TouchPhase, WindowEvent,
    },
};

/// A press that stays put this long breaks the block in view (Bedrock's touch default).
pub const HOLD: Duration = Duration::from_millis(300);
/// Holding pause this long opens the layout editor instead.
pub const EDIT_HOLD: Duration = Duration::from_millis(800);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Button {
    Jump,
    Sneak,
    Inventory,
    Chat,
    Pause,
    /// Held: key.attack (hit what is in view, break blocks).
    Attack,
    /// Held: key.use (eat, draw a bow, raise a shield, place).
    Use,
    Drop,
    Offhand,
    /// key.togglePerspective.
    Camera,
    /// key.command: the chat with a slash.
    Command,
    /// key.debug.overlay (F3): held as the key, so a tap shows or hides the debug screen. While
    /// it is held, its combinations ([`DEBUG_COMBOS`]) show beside it.
    Debug,
    /// key.pickItem: the block (or mob) in view into the hand (Bedrock's Pick Block button).
    Pick,
    /// key.playerlist, held: the list of the server's players.
    Players,
    /// Hotbar scroll: the previous slot (the mouse wheel one way).
    ScrollLeft,
    /// Hotbar scroll: the next slot (the mouse wheel the other way).
    ScrollRight,
}

/// F3's combinations offered while the F3 button is held: the key binding pressed with it and
/// what the chip says.
pub const DEBUG_COMBOS: [(&str, &str); 8] = [
    ("key.debug.showChunkBorders", "Chunks"),
    ("key.debug.showHitboxes", "Hitboxes"),
    ("key.debug.showAdvancedTooltips", "Tooltips"),
    ("key.debug.copyLocation", "Copy pos"),
    ("key.debug.reloadResourcePacks", "Packs"),
    ("key.debug.clearChat", "Clear chat"),
    ("key.debug.spectate", "Spectate"),
    ("key.debug.switchGameMode", "Modes"),
];

impl Button {
    pub const ALL: [Button; 16] = [
        Button::Jump,
        Button::Sneak,
        Button::Inventory,
        Button::Chat,
        Button::Pause,
        Button::Attack,
        Button::Use,
        Button::Drop,
        Button::Offhand,
        Button::Camera,
        Button::Command,
        Button::Debug,
        Button::Pick,
        Button::Players,
        Button::ScrollLeft,
        Button::ScrollRight,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Button::Jump => "Jump",
            Button::Sneak => "Sneak",
            Button::Inventory => "Bag",
            Button::Chat => "Chat",
            Button::Pause => "II",
            Button::Attack => "Hit",
            Button::Use => "Use",
            Button::Drop => "Drop",
            Button::Offhand => "Swap",
            Button::Camera => "Cam",
            Button::Command => "/",
            Button::Debug => "F3",
            Button::Pick => "Pick",
            Button::Players => "List",
            Button::ScrollLeft => "<",
            Button::ScrollRight => ">",
        }
    }
    /// Its name in touch-layout.json.
    pub fn id(self) -> &'static str {
        match self {
            Button::Jump => "jump",
            Button::Sneak => "sneak",
            Button::Inventory => "inventory",
            Button::Chat => "chat",
            Button::Pause => "pause",
            Button::Attack => "attack",
            Button::Use => "use",
            Button::Drop => "drop",
            Button::Offhand => "offhand",
            Button::Camera => "camera",
            Button::Command => "command",
            Button::Debug => "debug",
            Button::Pick => "pick",
            Button::Players => "players",
            Button::ScrollLeft => "scroll_left",
            Button::ScrollRight => "scroll_right",
        }
    }
    /// Only for aiming the crosshair: tap to interact has no Attack and Use buttons (Bedrock's
    /// "Joystick & tap to interact" shows none).
    fn aims(self) -> bool {
        matches!(self, Button::Attack | Button::Use)
    }
}

/// What a tap and a still press on the view act on: Bedrock's "tap to interact" (what is under
/// the finger: a tap hits a mob or uses a block, a press breaks the block or interacts with the
/// mob) or "aim crosshair" (what the crosshair is on: a tap uses, a press breaks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interact {
    #[default]
    Tap,
    Crosshair,
}

impl Interact {
    /// Its name in touch-layout.json.
    pub fn id(self) -> &'static str {
        match self {
            Interact::Tap => "tap",
            Interact::Crosshair => "crosshair",
        }
    }
    fn from_id(id: &str) -> Option<Self> {
        [Interact::Tap, Interact::Crosshair]
            .into_iter()
            .find(|i| i.id() == id)
    }
    /// What the editor's button says: what is touched, or what is aimed at (a press interacts in
    /// both, so not "tap").
    pub fn label(self) -> &'static str {
        match self {
            Interact::Tap => "Touch",
            Interact::Crosshair => "Aim",
        }
    }
    fn next(self) -> Self {
        match self {
            Interact::Tap => Interact::Crosshair,
            Interact::Crosshair => Interact::Tap,
        }
    }
}

/// Whether a control shows and works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    On,
    /// Not drawn, still works where it is (for players who know where it is).
    Ghost,
    /// Neither drawn nor working.
    Off,
}

impl Show {
    pub fn id(self) -> &'static str {
        match self {
            Show::On => "on",
            Show::Ghost => "ghost",
            Show::Off => "off",
        }
    }
    fn from_id(id: &str) -> Option<Self> {
        [Show::On, Show::Ghost, Show::Off]
            .into_iter()
            .find(|s| s.id() == id)
    }
    /// The editor's next state; the stick is never off.
    fn next(self, stick: bool) -> Self {
        match self {
            Show::On => Show::Ghost,
            Show::Ghost if stick => Show::On,
            Show::Ghost => Show::Off,
            Show::Off => Show::On,
        }
    }
}

/// How the Sneak button sneaks: while a finger is on it, or from one tap to the next (as it always
/// did). Flying, it only sneaks (goes down) while held either way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SneakMode {
    Hold,
    #[default]
    Toggle,
}

impl SneakMode {
    /// Its name in touch-layout.json.
    pub fn id(self) -> &'static str {
        match self {
            SneakMode::Hold => "hold",
            SneakMode::Toggle => "toggle",
        }
    }
    fn from_id(id: &str) -> Option<Self> {
        [SneakMode::Hold, SneakMode::Toggle]
            .into_iter()
            .find(|m| m.id() == id)
    }
    /// What the editor's chip says.
    pub fn label(self) -> &'static str {
        match self {
            SneakMode::Hold => "Hold",
            SneakMode::Toggle => "Toggle",
        }
    }
    /// The other one: the chip switches between them.
    fn next(self) -> Self {
        match self {
            SneakMode::Hold => SneakMode::Toggle,
            SneakMode::Toggle => SneakMode::Hold,
        }
    }
}

/// Where one control is: its centre as a share of the window's width and height (so a layout
/// fits any screen), its radius in units (a ninth of the shorter side), whether it shows, and
/// how opaque it is drawn (times the layout's own opacity).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub show: Show,
    pub opacity: f32,
}

const fn place(x: f32, y: f32, size: f32) -> Place {
    Place {
        x,
        y,
        size,
        show: Show::On,
        opacity: 1.0,
    }
}

/// A button off until the player turns it on in the editor (as Bedrock's optional ones).
const fn place_off(x: f32, y: f32, size: f32) -> Place {
    Place {
        show: Show::Off,
        ..place(x, y, size)
    }
}

/// The lowest opacity the editor sets: an invisible control is a ghost.
pub const MIN_OPACITY: f32 = 0.1;
/// The smallest a control gets, and a mod's button (a keybind's is as tall as a vanilla button,
/// less than that on a large screen).
const MIN_SIZE: f32 = 0.3;
const MIN_EXTRA_SIZE: f32 = 0.2;

/// How a button is shaped, which is also where a finger presses it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A circle, as the game's own buttons.
    Round,
    /// A rounded square, as Bedrock's.
    Square,
    /// This many times as wide as it is tall: a button with a label.
    Wide(f32),
}

impl Shape {
    /// Half its width and height, for its radius (half its height).
    pub fn half(self, r: f32) -> [f32; 2] {
        match self {
            Shape::Wide(aspect) => [r * aspect.max(1.0), r],
            Shape::Round | Shape::Square => [r, r],
        }
    }
    /// Whether `p` presses a button of this shape centred on `c`, a little past its edge too.
    pub fn hits(self, c: [f32; 2], r: f32, p: [f32; 2]) -> bool {
        match self {
            Shape::Round => dist(p, c) <= r * 1.15,
            _ => {
                let [hw, hh] = self.half(r);
                (p[0] - c[0]).abs() <= hw * 1.15 && (p[1] - c[1]).abs() <= hh * 1.15
            }
        }
    }
}

/// A button a mod put on the touch controls (one of its keybinds, or its own).
#[derive(Clone, Debug, PartialEq)]
pub struct Extra {
    /// `<mod id>:<button id>`: its name in touch-layout.json.
    pub key: String,
    pub place: Place,
    pub shape: Shape,
    /// Its mod runs and offers it now; otherwise it only keeps its place for when it does.
    pub live: bool,
}

/// The player's layout: the stick's resting place and every button's (the mods' too), and the
/// opacity of them all.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutConfig {
    pub stick: Place,
    /// Bedrock's D-pad in the stick's place (four arrows, the corners between them diagonals,
    /// up tapped twice to sprint) instead of the stick: always, never, or when none was picked
    /// (`None`) on a server where the stick moves as the keys. Its middle holds nothing yet.
    pub dpad: Option<bool>,
    pub buttons: Vec<(Button, Place)>,
    pub extra: Vec<Extra>,
    pub opacity: f32,
    /// How the Sneak button sneaks.
    pub sneak: SneakMode,
    /// What taps and presses on the view act on.
    pub interact: Interact,
    /// The stick stays where it rests ([`Layout::fixed_stick`]).
    pub fixed_stick: bool,
    /// The phone buzzes a moment when a block breaks and when a stack is split (Bedrock's
    /// "Vibrate when breaking blocks" and "when splitting items", both on by default).
    pub vibrate: bool,
    /// Turning the phone turns the view too ([`gyro_turn`]), as PojavLauncher's gyroscope.
    pub gyro: bool,
}

/// The view's turn, in degrees right and down, for what the gyroscope turned (radians about
/// the device's own x and y, those of its natural portrait) on a display at `rotation`
/// (Surface.ROTATION_0..3: 1 with the phone's top to the left). The phone is a window on the
/// world: turned left about the screen's up, the view turns left; its top tilted toward the
/// player (its back to the sky), the view looks up.
pub fn gyro_turn(turned: [f32; 2], rotation: i32) -> [f32; 2] {
    let [x, y] = turned;
    // Rotation about the screen's right and up axes.
    let (right, up) = match rotation.rem_euclid(4) {
        0 => (x, y),
        1 => (-y, x),
        2 => (-x, -y),
        _ => (y, -x),
    };
    [-up.to_degrees(), -right.to_degrees()]
}

impl Default for LayoutConfig {
    /// Laid out on a 20:9 phone: the stick at the bottom left, jump, attack, use and sneak at the
    /// bottom right, the menus along the top, drop and offhand beside the hotbar.
    fn default() -> Self {
        LayoutConfig {
            stick: place(0.12, 0.733, 1.3),
            dpad: None,
            buttons: vec![
                (Button::Jump, place(0.905, 0.7556, 0.8)),
                (Button::Sneak, place(0.805, 0.8556, 0.6)),
                (Button::Attack, place(0.815, 0.611, 0.6)),
                (Button::Use, place(0.92, 0.533, 0.6)),
                (Button::Inventory, place(0.95, 0.111, 0.55)),
                (Button::Chat, place(0.88, 0.111, 0.55)),
                (Button::Command, place(0.81, 0.111, 0.5)),
                (Button::Pause, place(0.5, 0.0833, 0.45)),
                (Button::Camera, place(0.43, 0.0833, 0.45)),
                (Button::Debug, place(0.57, 0.0833, 0.45)),
                (Button::Drop, place(0.711, 0.951, 0.4)),
                (Button::Offhand, place(0.289, 0.951, 0.4)),
                (Button::Pick, place_off(0.83, 0.4, 0.5)),
                (Button::Players, place_off(0.36, 0.0833, 0.45)),
                (Button::ScrollLeft, place(0.36, 0.84, 0.45)),
                (Button::ScrollRight, place(0.64, 0.84, 0.45)),
            ],
            extra: Vec::new(),
            opacity: 1.0,
            sneak: SneakMode::Toggle,
            interact: Interact::Tap,
            fixed_stick: false,
            vibrate: true,
            gyro: false,
        }
    }
}

impl LayoutConfig {
    /// touch-layout.json; controls it does not name keep their default place (a layout from
    /// before a button existed), and names it does not know are skipped.
    pub fn from_json(text: &str) -> Self {
        let mut config = LayoutConfig::default();
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return config;
        };
        let read = |v: &Value, into: &mut Place, min_size: f32| {
            let num = |k: &str, d: f32| v[k].as_f64().map_or(d, |n| n as f32);
            // The first layouts said `visible` before ghosts existed.
            let legacy = v["visible"]
                .as_bool()
                .map(|on| if on { Show::On } else { Show::Off });
            *into = Place {
                x: num("x", into.x).clamp(0.0, 1.0),
                y: num("y", into.y).clamp(0.0, 1.0),
                size: num("size", into.size).clamp(min_size, 2.5),
                show: v["show"]
                    .as_str()
                    .and_then(Show::from_id)
                    .or(legacy)
                    .unwrap_or(into.show),
                opacity: num("opacity", into.opacity).clamp(MIN_OPACITY, 1.0),
            };
        };
        if let Some(opacity) = value["opacity"].as_f64() {
            config.opacity = (opacity as f32).clamp(MIN_OPACITY, 1.0);
        }
        // "stick" was what an unpicked layout saved before "auto".
        config.dpad = match value["movement"].as_str() {
            Some("dpad") => Some(true),
            Some("analog") => Some(false),
            _ => None,
        };
        // Layouts from before the choice toggle, as Sneak did then.
        config.sneak = value["sneak"]
            .as_str()
            .and_then(SneakMode::from_id)
            .unwrap_or_default();
        // Layouts from before the choice get Bedrock's default too.
        config.interact = value["interact"]
            .as_str()
            .and_then(Interact::from_id)
            .unwrap_or_default();
        config.fixed_stick = value["anchor"].as_str() == Some("fixed");
        config.vibrate = value["vibrate"].as_bool().unwrap_or(true);
        config.gyro = value["gyro"].as_bool().unwrap_or(false);
        if value["stick"].is_object() {
            read(&value["stick"], &mut config.stick, MIN_SIZE);
            if config.stick.show == Show::Off {
                config.stick.show = Show::On;
            }
        }
        for (button, place) in &mut config.buttons {
            if value["buttons"][button.id()].is_object() {
                read(&value["buttons"][button.id()], place, MIN_SIZE);
            }
        }
        // The mods' buttons, those of mods not running now too (they keep their places).
        if let Some(mods) = value["mods"].as_object() {
            for (key, v) in mods.iter().filter(|(_, v)| v.is_object()).take(256) {
                let mut place = place(0.5, 0.5, 0.5);
                read(v, &mut place, MIN_EXTRA_SIZE);
                config.extra.push(Extra {
                    key: key.clone(),
                    place,
                    shape: Shape::Round,
                    live: false,
                });
            }
        }
        config
    }

    pub fn to_json(&self) -> String {
        let entry = |p: &Place| json!({ "x": p.x, "y": p.y, "size": p.size, "show": p.show.id(), "opacity": p.opacity });
        let buttons: Map<String, Value> = self
            .buttons
            .iter()
            .map(|(b, p)| (b.id().to_owned(), entry(p)))
            .collect();
        let mods: Map<String, Value> = self
            .extra
            .iter()
            .map(|e| (e.key.clone(), entry(&e.place)))
            .collect();
        let value = json!({
            "version": 2,
            "opacity": self.opacity,
            "movement": match self.dpad {
                Some(true) => "dpad",
                Some(false) => "analog",
                None => "auto",
            },
            "sneak": self.sneak.id(),
            "interact": self.interact.id(),
            "anchor": if self.fixed_stick { "fixed" } else { "floating" },
            "vibrate": self.vibrate,
            "gyro": self.gyro,
            "stick": entry(&self.stick),
            "buttons": buttons,
            "mods": mods,
        });
        serde_json::to_string_pretty(&value).unwrap_or_default()
    }

    /// The mods' buttons this frame (`<mod id>:<button id>`, the place its mod gives it, its
    /// shape): each keeps the place the player gave it, if any; the rest are no longer live.
    pub fn offer(&mut self, offered: &[(String, Place, Shape)]) {
        for e in &mut self.extra {
            e.live = false;
        }
        for (key, place, shape) in offered {
            match self.extra.iter_mut().find(|e| e.key == *key) {
                Some(e) => {
                    e.live = true;
                    e.shape = *shape;
                }
                None => self.extra.push(Extra {
                    key: key.clone(),
                    place: *place,
                    shape: *shape,
                    live: true,
                }),
            }
        }
    }

    /// Whether `p` presses `target`, drawn centred on `c` with radius `r`.
    fn hits(&self, target: Target, c: [f32; 2], r: f32, p: [f32; 2]) -> bool {
        let shape = match target {
            Target::Extra(i) => self.extra.get(i).map_or(Shape::Round, |e| e.shape),
            Target::Stick | Target::Button(_) => 
