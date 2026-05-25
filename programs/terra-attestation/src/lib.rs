use anchor_lang::prelude::*;

declare_id!("2NHpzDTyF4Z1qm3oHRDM8vKjouU1SSHYtof7L7rVHpWV");

#[program]
pub mod terra_attestation {
    use super::*;

    pub fn initialize(_ctx: Context<Initialize>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
