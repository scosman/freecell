//! `Rgb` — a plain 24-bit colour, the one colour type the whole app passes around.
//!
//! Fills, font colours, and the palette are all `Rgb`. It is engine-free and
//! GPUI-free: the grid maps it onto `gpui::rgb(...)` at draw time, and the engine
//! adapter maps IronCalc's colour strings onto it. Its shape mirrors the frozen
//! `datagen::Rgb` (`experiments/shared/datagen/src/cell.rs`) — copied, not referenced,
//! because the app must not depend on throwaway experiment crates (`architecture.md §1`).

/// A 24-bit RGB colour (`0xRRGGBB` when packed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    /// Builds a colour from its three channels.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Unpacks a `0xRRGGBB` value (the form `gpui::rgb` and the UI-design hexes use).
    /// The top byte is ignored.
    pub const fn from_hex(hex: u32) -> Self {
        Self {
            r: ((hex >> 16) & 0xFF) as u8,
            g: ((hex >> 8) & 0xFF) as u8,
            b: (hex & 0xFF) as u8,
        }
    }

    /// Packs the colour into a `0xRRGGBB` `u32`.
    pub const fn to_hex(self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_hex_roundtrip() {
        for hex in [0x000000u32, 0xFFFFFF, 0x4472C4, 0x123456, 0xED7D31] {
            assert_eq!(
                Rgb::from_hex(hex).to_hex(),
                hex,
                "roundtrip failed for {hex:#08X}"
            );
        }
        // Channels unpack in the expected positions.
        let c = Rgb::from_hex(0x12_34_56);
        assert_eq!((c.r, c.g, c.b), (0x12, 0x34, 0x56));
        // The high byte is dropped on unpack.
        assert_eq!(Rgb::from_hex(0xFF_12_34_56), Rgb::new(0x12, 0x34, 0x56));
    }
}
