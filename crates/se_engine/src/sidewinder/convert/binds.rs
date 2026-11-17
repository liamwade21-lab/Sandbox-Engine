//! Elite's two key files, copied byte for byte from the install (`Ctx::install`'s `ControlSchemes`): its one job.
//! `binds/KeyboardMouseOnly.binds` and `binds/ControlPad.binds` are the key tables the seat reads (grade A);
//! `bindings_match_elite` re-checks them against the install.

use se_core::elite_asset::Binds;

use super::{Ctx, Shipped};

/// The key files, as Elite names them.
pub(super) const FILES: [&str; 2] = ["KeyboardMouseOnly.binds", "ControlPad.binds"];

/// Copies the key files and reads each back as the seat reads it.
pub(super) fn convert(ctx: &Ctx, shipped: &mut Shipped) -> Result<(), String> {
    for b in FILES {
        let rel = format!("binds/{b}");
        let from = ctx.install.join("ControlSchemes").join(b);
        ctx.write(&rel, &std::fs::read(&from).map_err(|e| format!("{}: {e}", from.display()))?)?;
        Binds::parse(&String::from_utf8_lossy(&ctx.read(&rel)?)).map_err(|e| format!("{rel}: {e}"))?;
        shipped.add(&rel, &[format!("Elite install ControlSchemes/{b}")], "A", "A copy of Elite's key file.");
    }
    Ok(())
}
