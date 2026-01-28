use strata_primitives::OLBlockId;

use crate::{BlockSigningDuty, Duty, Error, TemplateManager};

/// Extract sequencer duties
pub async fn extract_duties(
    template_mgr: &TemplateManager,
    tip_blkid: OLBlockId,
    // TODO: add params required for checkpoint duties
) -> Result<Vec<Duty>, Error> {
    let mut duties = vec![];
    let template = template_mgr.generate_template(tip_blkid).await?;
    let blkduty = BlockSigningDuty::new(template);
    duties.push(Duty::SignBlock(blkduty));
    // TODO: add checkpoint duties, wait for checkpoint db to be completed in next PR
    Ok(duties)
}
