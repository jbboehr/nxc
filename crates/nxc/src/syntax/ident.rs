// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

// Temporary nxc spellings for native variable and parameter names. Attribute
// keys are literal names and must never pass through this mapping.
const COMPATIBILITY_NAMES: [(&str, &str); 2] =
    [("fn", "__nxc_ident_fn"), ("yield", "__nxc_ident_yield")];

pub(crate) fn encode(name: &str) -> &str {
    COMPATIBILITY_NAMES
        .iter()
        .find_map(|&(native, nxc)| (name == native).then_some(nxc))
        .unwrap_or(name)
}

pub(crate) fn decode(name: &str) -> &str {
    COMPATIBILITY_NAMES
        .iter()
        .find_map(|&(native, nxc)| (name == nxc).then_some(native))
        .unwrap_or(name)
}
