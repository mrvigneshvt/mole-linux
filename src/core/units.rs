//! Byte-size labels shared by clean, purge and installer, plus the IEC parser for the
//! pacman and paccache reports. Ported from tw93/Mole `internal/units/bytes.go` (`BytesSI`).
//!
//! Disk sizes use SI (1000-based) units. Upstream rounds to one decimal; this port truncates,
//! as the clean, purge and orphan output already did, so a label never overstates the space.

const SI_UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];

/// SI label for a disk size: `999 B`, `1.0 kB`, `999.9 kB`, `18.4 EB`.
pub fn bytes_si(bytes: u64) -> String {
    if bytes < 1_000 {
        return format!("{bytes} B");
    }
    let mut unit = 0;
    let mut divisor = 1_000_u64;
    while unit + 1 < SI_UNITS.len() && bytes / divisor >= 1_000 {
        divisor *= 1_000;
        unit += 1;
    }
    let tenths = bytes / (divisor / 10);
    format!("{}.{} {}", tenths / 10, tenths % 10, SI_UNITS[unit])
}

/// Parses an IEC figure such as `510.4 KiB` (paccache, pacman) into bytes.
/// Integer arithmetic only; anything that does not fit `u64` or uses another unit is `None`.
pub fn parse_iec_size(value: &str) -> Option<u64> {
    let (number, unit) = value.split_once(' ')?;
    let multiplier: u64 = match unit {
        "B" => 1,
        "KiB" => 1 << 10,
        "MiB" => 1 << 20,
        "GiB" => 1 << 30,
        "TiB" => 1 << 40,
        _ => return None,
    };
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    let whole: u64 = whole.parse().ok()?;
    let fractional = if fraction.is_empty() {
        0
    } else {
        let digits: u64 = fraction.parse().ok()?;
        let scale = 10_u64.checked_pow(u32::try_from(fraction.len()).ok()?)?;
        digits.checked_mul(multiplier)? / scale
    };
    whole.checked_mul(multiplier)?.checked_add(fractional)
}
