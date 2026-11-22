//! Which component reads each file format of the suite, so the tree can say whose a file is.
//!
//! Each format belongs to the reader that states it (`docs/*-schema.md`, the plans). This table
//! repeats what those readers say for the tree's labels only: it decides nothing about a file,
//! which is still read and refused by its own reader. A format missing here is shown by its stamp
//! with no component.
//!
//! TOML formats only: comissionador's test and function files open with a stamp too
//! (`veredas-test`, `veredas-functions`, docs/test-schema.md) but are written in a grammar of their
//! own, read by their own reader, and the tree shows them as plain files until their names say
//! what they are.

/// A component of the suite as the tree names it: its ASCII `nome` and the name screens show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Component {
    pub nome: &'static str,
    pub display: &'static str,
}

const CORE: Component = Component { nome: "core", display: "Core" };
const PORTAL: Component = Component { nome: "portal", display: "Portal" };
const MAQUINAR: Component = Component { nome: "maquinar", display: "Maquinar" };
const SUPERVISAO: Component = Component { nome: "supervisao", display: "Supervisão" };
/// Suite releases: no component, but the files are the suite's (docs/entrega-plan.md).
const ENTREGA: Component = Component { nome: "entrega", display: "Entrega" };

/// Every TOML format the suite's documents state today, with its reader's component.
pub const FORMATS: &[(&str, Component)] = &[
    ("veredas-project", PORTAL),
    ("veredas-secrets", PORTAL),
    ("veredas-channel", CORE),
    ("veredas-catalog", CORE),
    ("veredas-bench", MAQUINAR),
    ("veredas-controller", MAQUINAR),
    ("veredas-supervisao", SUPERVISAO),
    ("veredas-binding", SUPERVISAO),
    ("veredas-release", ENTREGA),
];

/// The component whose reader states `format`, if the table knows it.
pub fn component_of(format: &str) -> Option<Component> {
    FORMATS.iter().find(|(f, _)| *f == format).map(|(_, c)| *c)
}
