//! #81: one SI byte formatter for clean, purge and installer, and one IEC parser for
//! the pacman/paccache reports. Ported from tw93/Mole internal/units/bytes.go (BytesSI),
//! with truncation to one decimal to match the existing clean, purge and orphan output.

use mole_linux::core::units::{bytes_si, parse_iec_size};

#[test]
fn si_keeps_plain_bytes_below_one_thousand() {
    assert_eq!(bytes_si(0), "0 B");
    assert_eq!(bytes_si(999), "999 B");
}

#[test]
fn si_promotes_exactly_at_each_power_of_one_thousand() {
    assert_eq!(bytes_si(1_000), "1.0 kB");
    assert_eq!(bytes_si(1_000_000), "1.0 MB");
    assert_eq!(bytes_si(1_000_000_000), "1.0 GB");
    assert_eq!(bytes_si(1_000_000_000_000), "1.0 TB");
    assert_eq!(bytes_si(1_000_000_000_000_000), "1.0 PB");
    assert_eq!(bytes_si(1_000_000_000_000_000_000), "1.0 EB");
}

#[test]
fn si_never_prints_a_thousand_in_a_smaller_unit() {
    // 999 999 B is 999.9 kB; rounding up would print "1000.0 kB".
    assert_eq!(bytes_si(999_999), "999.9 kB");
    assert_eq!(bytes_si(999_999_999), "999.9 MB");
}

#[test]
fn si_truncates_to_one_decimal() {
    assert_eq!(bytes_si(1_234_567), "1.2 MB");
    assert_eq!(bytes_si(1_500_000), "1.5 MB");
    assert_eq!(bytes_si(524_288), "524.2 kB");
    assert_eq!(bytes_si(1_579_996_296), "1.5 GB");
}

#[test]
fn si_covers_the_whole_u64_range() {
    assert_eq!(bytes_si(u64::MAX), "18.4 EB");
}

#[test]
fn iec_parser_reads_pacman_and_paccache_reports() {
    // paccache: "==> finished dry run: 1 candidates (disk space saved: 510.4 KiB)"
    assert_eq!(parse_iec_size("510.4 KiB"), Some(522_649));
    assert_eq!(parse_iec_size("16 KiB"), Some(16_384));
    assert_eq!(parse_iec_size("1.50 MiB"), Some(1_572_864));
    assert_eq!(parse_iec_size("2 GiB"), Some(2_147_483_648));
    assert_eq!(parse_iec_size("512 B"), Some(512));
}

#[test]
fn iec_parser_refuses_unknown_units_and_overflow() {
    assert_eq!(parse_iec_size("3 GB"), None);
    assert_eq!(parse_iec_size("1.5 PiB"), None);
    assert_eq!(parse_iec_size("18446744073709551615 TiB"), None);
    assert_eq!(parse_iec_size("fast"), None);
}
