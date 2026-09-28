// SPDX-License-Identifier: GPL-3.0-only

// WMDE: how much memory a decode may take, as wmde-photos decided it. A decode is checked
// before it starts, against the peak it will reach rather than the size of what it produces,
// and a picture that does not fit is refused with a reason. Without it the decoder asked for
// whatever the header said, and a picture larger than memory killed the viewer or pushed the
// session into swap. Decodes running at once reserve their share, so neighbours loaded ahead
// together do not each count the same free memory.

use crate::{LoadError, loader::MAX_TEX};
use std::sync::{Mutex, PoisonError};

/// Left for the rest of the machine. A viewer that wins the memory it wanted by pushing the
/// session into swap has not won anything.
const RESERVE: u64 = 400 * 1_000_000;

/// A floor, so a machine under pressure still opens ordinary photographs instead of refusing
/// everything.
const FLOOR: u64 = 256 * 1_000_000;

/// The decodes running now: what they have reserved, and `MemAvailable` as it was when the
/// first of them started.
struct Running {
    held: u64,
    baseline: Option<u64>,
}

static RUNNING: Mutex<Running> = Mutex::new(Running {
    held: 0,
    baseline: None,
});

/// Memory held for one decode; given back when dropped.
#[must_use = "the reservation lasts as long as it is held"]
pub struct Reservation(u64);

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut running = RUNNING.lock().unwrap_or_else(PoisonError::into_inner);
        running.held = running.held.saturating_sub(self.0);
    }
}

/// What one decode may allocate now, before the ones running are taken off.
pub fn for_decode() -> u64 {
    within(available())
}

/// Hold `needed` bytes for a decode, or refuse it when they are not there.
///
/// # Errors
///
/// [`LoadError::TooLarge`] with what is free for it.
pub fn reserve(needed: u64) -> Result<Reservation, LoadError> {
    let mut running = RUNNING.lock().unwrap_or_else(PoisonError::into_inner);
    admit(&mut running, available(), needed).map(|()| Reservation(needed))
}

/// Count `needed` in with the decodes running, `live` being `MemAvailable` now.
///
/// What the running decodes have allocated so far is gone from `live` already, and they are
/// counted again through what they reserved: it is added back, but never above what was free
/// when the first of them started, so neighbours starting together do not each count the same
/// free memory, and what other programs took meanwhile still counts.
fn admit(running: &mut Running, live: Option<u64>, needed: u64) -> Result<(), LoadError> {
    if running.held == 0 {
        running.baseline = live;
    }
    let free_memory = match (live, running.baseline) {
        (Some(live), Some(baseline)) => Some(live.saturating_add(running.held).min(baseline)),
        (live, _) => live,
    };
    let free = within(free_memory).saturating_sub(running.held);
    if needed > free {
        return Err(LoadError::TooLarge {
            needed,
            available: free,
        });
    }
    running.held += needed;
    Ok(())
}

/// The peak of a decode of `width` x `height` at `bytes_per_pixel`: `copies` of the picture,
/// as many as the decoder's path holds at once, and the texture made from it.
pub fn peak(width: u32, height: u32, bytes_per_pixel: u64, copies: u64, alpha: bool) -> u64 {
    let picture = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(bytes_per_pixel);
    picture
        .saturating_mul(copies)
        .saturating_add(texture(width, height, bytes_per_pixel, alpha))
}

/// The texture: RGBA no larger than `MAX_TEX` a side. A larger picture goes through the
/// resizer, which holds an intermediate `MAX_TEX` across the picture's longer side at its own
/// depth and, for a picture with alpha, a whole copy multiplied by it.
fn texture(width: u32, height: u32, bytes_per_pixel: u64, alpha: bool) -> u64 {
    let max = u64::from(MAX_TEX);
    let (width, height) = (u64::from(width), u64::from(height));
    let rgba = width.min(max) * height.min(max) * 4;
    if width <= max && height <= max {
        return rgba;
    }
    let intermediate = width.max(height) * max * bytes_per_pixel;
    let premultiplied = if alpha {
        width.saturating_mul(height).saturating_mul(bytes_per_pixel)
    } else {
        0
    };
    rgba.saturating_add(intermediate)
        .saturating_add(premultiplied)
}

fn within(available: Option<u64>) -> u64 {
    available.map_or(FLOOR, |free| free.saturating_sub(RESERVE).max(FLOOR))
}

/// `MemAvailable` in bytes: the kernel's own estimate of what can be had without swapping.
fn available() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_available(&meminfo)
}

fn parse_available(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let kib: u64 = line
            .strip_prefix("MemAvailable:")?
            .split_whitespace()
            .next()?
            .parse()
            .ok()?;
        kib.checked_mul(1024)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_available_is_read_in_bytes() {
        let meminfo = "MemTotal:        3979124 kB\nMemAvailable:    3035640 kB\nBuffers: 1 kB\n";
        assert_eq!(parse_available(meminfo), Some(3_035_640 * 1024));
        assert_eq!(parse_available("MemTotal: 100 kB\n"), None);
    }

    #[test]
    fn the_budget_keeps_a_reserve_and_never_falls_below_the_floor() {
        assert_eq!(within(Some(2_000_000_000)), 1_600_000_000);
        assert_eq!(within(Some(500_000_000)), FLOOR);
        assert_eq!(within(None), FLOOR);
        assert!(for_decode() >= FLOOR);
    }

    #[test]
    fn decodes_running_at_once_share_the_budget_and_count_once() {
        const MB: u64 = 1_000_000;
        let mut running = Running {
            held: 0,
            baseline: None,
        };
        // 1000 MB free: 600 for a decode.
        admit(&mut running, Some(1000 * MB), 300 * MB).expect("fits");
        // A neighbour starting before the first allocated: the same free memory, less the 300.
        match admit(&mut running, Some(1000 * MB), 400 * MB) {
            Err(LoadError::TooLarge { needed, available }) => {
                assert_eq!((needed, available), (400 * MB, 300 * MB));
            }
            _ => panic!("the neighbour is refused"),
        }
        // Once the first has allocated its 300, it is not counted a second time.
        admit(&mut running, Some(700 * MB), 250 * MB).expect("fits beside the first");
        assert_eq!(running.held, 550 * MB);
        // Another program taking memory meanwhile still counts.
        assert!(admit(&mut running, Some(200 * MB), 60 * MB).is_err());
    }

    #[test]
    fn the_peak_counts_the_copies_and_the_texture() {
        // Within the texture size: the picture and its RGBA texture.
        assert_eq!(peak(100, 50, 3, 1, false), 100 * 50 * 3 + 100 * 50 * 4);
        // Above it: a texture capped at MAX_TEX and the resizer's intermediate, and with alpha
        // the whole premultiplied copy.
        let max = u64::from(MAX_TEX);
        assert_eq!(
            peak(8000, 4000, 3, 2, false),
            8000 * 4000 * 3 * 2 + max * max * 4 + 8000 * max * 3
        );
        assert_eq!(
            peak(8000, 4000, 4, 1, true),
            8000 * 4000 * 4 * 2 + max * max * 4 + 8000 * max * 4
        );
        assert_eq!(peak(u32::MAX, u32::MAX, 8, 3, true), u64::MAX);
    }
}
