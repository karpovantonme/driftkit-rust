//! `driftkit mcp`: an MCP tool's input schema against what its handler reads.
//!
//! The schema declares a property the handler never looks at, or the handler
//! reads a key the schema never declares. The agent obeys the schema, so
//! either a capability is silently unreachable or the call fails outright.
//!
//! Measured on 265 servers in August 2026: 39 susceptible, and the findings
//! come in batches -- one KiCAD server had 15 tools that could not work at
//! all, because their schemas required `layerName` while the handlers read
//! `layer`.
//!
//! 🔴 Susceptibility is decided before scanning, not after filtering. Where a
//! single source feeds both the schema and the handler -- FastMCP, zod with
//! `z.infer`, `zodToJsonSchema` -- the mismatch is inexpressible, and the
//! server is skipped rather than scanned and filtered. That removed 60% of
//! the work.

pub mod classify;
