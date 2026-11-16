//! Which game modes and maps this server hosts (`--only-modes`,
//! `--exclude-modes`, `--exclude-maps`). Set once at startup; the engine
//! refuses a session for anything filtered out, and the built-in matchmaker
//! never creates one.

use std::sync::OnceLock;

use crate::sns::symbol;

/// Game modes by their game-type name, with a short label.
pub const MODES: &[(&str, &str)] = &[
    ("echo_arena", "Arena (public)"),
    ("echo_arena_private", "Arena (private)"),
    ("echo_arena_public_ai", "Arena vs AI"),
    ("echo_arena_practice", "Arena practice"),
    ("echo_combat", "Combat (public)"),
    ("echo_combat_private", "Combat (private)"),
    ("social_2.0", "Social lobby"),
    ("social_2.0_private", "Social lobby (private)"),
];

/// Maps (levels) by name, with a short label.
pub const MAPS: &[(&str, &str)] = &[
    ("mpl_arena_a", "Arena"),
    ("mpl_lobby_b2", "Lobby"),
    ("mpl_combat_dyson", "Combat: Dyson"),
    ("mpl_combat_combustion", "Combat: Combustion"),
    ("mpl_combat_fission", "Combat: Fission"),
    ("mpl_combat_gauss", "Combat: Gauss"),
];

#[derive(Default, Debug)]
pub struct Filter {
    /// Empty: every mode not excluded.
    pub only_modes: Vec<u64>,
    pub exclude_modes: Vec<u64>,
    pub exclude_maps: Vec<u64>,
}

static FILTER: OnceLock<Filter> = OnceLock::new();

/// A mode or map name, or a raw `0x…` symbol, as its symbol.
fn parse(name: &str) -> anyhow::Result<u64> {
    let n = name.trim();
    if let Some(h) = n.strip_prefix("0x") {
        return Ok(u64::from_str_radix(h, 16)?);
    }
    anyhow::ensure!(!n.is_empty(), "empty mode or map name");
    Ok(symbol(n))
}

pub fn set(only_modes: &[String], exclude_modes: &[String], exclude_maps: &[String]) -> anyhow::Result<()> {
    let all = |v: &[String]| v.iter().filter(|s| !s.trim().is_empty()).map(|s| parse(s)).collect::<anyhow::Result<Vec<_>>>();
    let f = Filter { only_modes: all(only_modes)?, exclude_modes: all(exclude_modes)?, exclude_maps: all(exclude_maps)? };
    let _ = FILTER.set(f);
    Ok(())
}

pub fn allows(mode: u64, level: u64) -> bool {
    let Some(f) = FILTER.get() else { return true };
    (f.only_modes.is_empty() || f.only_modes.contains(&mode)) && !f.exclude_modes.contains(&mode) && !f.exclude_maps.contains(&level)
}

pub fn mode_name(mode: u64) -> String {
    MODES.iter().find(|(n, _)| symbol(n) == mode).map(|(n, _)| n.to_string()).unwrap_or_else(|| format!("{mode:#018x}"))
}

pub fn level_name(level: u64) -> String {
    MAPS.iter().find(|(n, _)| symbol(n) == level).map(|(n, _)| n.to_string()).unwrap_or_else(|| format!("{level:#018x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_names_hash_to_the_engine_symbols() {
        // The arena level the engine checks for, and the private arena mode
        // seen in captures.
        assert_eq!(symbol("mpl_arena_a"), 0x576e_d3f8_428e_bc4b);
        assert_eq!(symbol("echo_arena_private"), 0x0999_0965_f4db_8c03);
    }
}
