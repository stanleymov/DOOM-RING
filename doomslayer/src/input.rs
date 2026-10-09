//! Raw keyboard/mouse with per-frame edge detection. Only live while the game window is focused and
//! no menu has the cursor visible. The bridge can inject synthetic presses for automated tests.
//! Controller buttons come in as their own codes (gamepad.rs, 0x200..), so every check below works
//! the same for a key or a pad button; the left stick adds to WASD.

use std::collections::HashSet;

use windows::Win32::{
    System::Threading::GetCurrentProcessId,
    UI::{
        Input::KeyboardAndMouse::GetAsyncKeyState,
        WindowsAndMessaging::{
            CURSOR_SHOWING, CURSORINFO, GetCursorInfo, GetForegroundWindow, GetWindowThreadProcessId,
        },
    },
};

#[derive(Default)]
pub struct Input {
    down: HashSet<u16>,
    prev: HashSet<u16>,
    /// Synthetic presses from the test bridge, consumed next frame.
    injected: HashSet<u16>,
    injected_held: HashSet<u16>,
    /// Keys that were down while input was suppressed: ignored until released (the click that
    /// closes a menu must not fire the gun).
    latched: HashSet<u16>,
    pub active: bool,
    /// Left / right stick this frame ((x, y), dead zone out; zero while input is suppressed).
    pub lstick: (f32, f32),
    pub rstick: (f32, f32),
    /// Seconds (about) since the pad was last touched.
    pad_idle: f32,
}

pub fn game_has_focus() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}

pub fn cursor_visible() -> bool {
    let mut info = CURSORINFO {
        cbSize: size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetCursorInfo(&mut info).is_ok() && info.flags.0 & CURSOR_SHOWING.0 != 0 }
}

impl Input {
    pub fn update(&mut self, watched: &[u16]) {
        self.prev = std::mem::take(&mut self.down);
        self.active = game_has_focus() && !cursor_visible();
        let pad = crate::gamepad::poll();
        self.lstick = (0.0, 0.0);
        self.rstick = (0.0, 0.0);
        if self.active {
            let mut keys = false;
            for &vk in watched {
                if crate::gamepad::is_pad(vk) {
                    if pad.down(vk) {
                        self.down.insert(vk);
                    }
                } else if vk < 0x100 && unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0 {
                    self.down.insert(vk);
                    keys |= !self.prev.contains(&vk);
                }
            }
            self.lstick = pad.left();
            self.rstick = pad.right();
            // keyboard / mouse or pad mode: whichever was used last (HUD tags, binding screen)
            // (a real mouse move, not a few counts - Steam Input can nudge the mouse - and the pad
            // left alone for half a second: the mode flickered while playing on the pad)
            let mouse = crate::remap::MOUSE_ACT.swap(0, std::sync::atomic::Ordering::Relaxed) > 40;
            if pad.active() {
                self.pad_idle = 0.0;
            } else {
                self.pad_idle += 1.0 / 60.0;
            }
            if pad.active() && !crate::gamepad::pad_mode() {
                crate::gamepad::PAD_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
                log::info!("input: controller");
            } else if (keys || mouse) && self.pad_idle > 0.5 && crate::gamepad::pad_mode() {
                crate::gamepad::PAD_MODE.store(false, std::sync::atomic::Ordering::Relaxed);
                log::info!("input: keyboard / mouse");
            }
        }
        self.down.extend(self.injected.drain());
        self.down.extend(self.injected_held.iter().copied());
        let down = &self.down;
        self.latched.retain(|k| down.contains(k));
        for k in &self.latched {
            self.down.remove(k);
        }
    }

    /// Nothing counts as pressed this frame (menus, loading screens, cutscenes).
    pub fn suppress(&mut self) {
        self.latched.extend(self.down.drain());
        self.prev.clear();
        self.lstick = (0.0, 0.0);
        self.rstick = (0.0, 0.0);
    }

    pub fn inject(&mut self, vk: u16) {
        self.injected.insert(vk);
    }

    pub fn hold(&mut self, vk: u16, on: bool) {
        if on {
            self.injected_held.insert(vk);
        } else {
            self.injected_held.remove(&vk);
        }
    }

    pub fn down(&self, vk: u16) -> bool {
        self.down.contains(&vk)
    }

    pub fn pressed(&self, vk: u16) -> bool {
        self.down.contains(&vk) && !self.prev.contains(&vk)
    }

    pub fn released(&self, vk: u16) -> bool {
        !self.down.contains(&vk) && self.prev.contains(&vk)
    }

    /// Either of an action's two keys (0 = unbound).
    pub fn down2(&self, a: u16, b: u16) -> bool {
        (a != 0 && self.down(a)) || (b != 0 && self.down(b))
    }

    pub fn pressed2(&self, a: u16, b: u16) -> bool {
        // a press of one while the other is held still counts (two keys, one action)
        (a != 0 && self.pressed(a)) || (b != 0 && self.pressed(b))
    }

    /// Let go: neither key is down any more, and one was last frame.
    pub fn released2(&self, a: u16, b: u16) -> bool {
        !self.down2(a, b) && ((a != 0 && self.prev.contains(&a)) || (b != 0 && self.prev.contains(&b)))
    }

    /// An action's two keys and its controller button.
    pub fn down3(&self, a: u16, b: u16, pad: u16) -> bool {
        self.down2(a, b) || (pad != 0 && self.down(pad))
    }

    pub fn pressed3(&self, a: u16, b: u16, pad: u16) -> bool {
        self.pressed2(a, b) || (pad != 0 && self.pressed(pad))
    }

    pub fn released3(&self, a: u16, b: u16, pad: u16) -> bool {
        !self.down3(a, b, pad) && [a, b, pad].iter().any(|&k| k != 0 && self.prev.contains(&k))
    }
}

/// WASD and the left stick as a (strafe, forward) pair in [-1, 1].
pub fn move_axes(i: &Input) -> (f32, f32) {
    let ax = |pos: u16, neg: u16| (i.down(pos) as i32 - i.down(neg) as i32) as f32;
    let (x, y) = (ax(0x44, 0x41) + i.lstick.0, ax(0x57, 0x53) + i.lstick.1);
    (x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0))
}

pub const MOVE_KEYS: [u16; 4] = [0x57, 0x41, 0x53, 0x44];
