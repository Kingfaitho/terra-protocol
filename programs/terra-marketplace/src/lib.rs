use anchor_lang::prelude::*;

declare_id!("E2SKqnwvJqfCjveiM4vHEF5dr9yKVT3JLno6ghsvcDZx");

#[program]
pub mod terra_marketplace {
    use super::*;

    pub fn initialize(_ctx: Context<Initialize>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
