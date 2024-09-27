use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};  
use solana_program::sysvar::clock::Clock;

declare_id!("GRbcvLa6oYsjh28iSkLaFff1pkjdtAc9ybNRronynnyc");

#[program]
pub mod asset_backed_loans {
    use super::*;

    // Instruction to deposit collateral into a PDA account
    pub fn deposit_collateral(ctx: Context<DepositCollateral>, amount: u64) -> Result<()> {
        let collateral_account = &mut ctx.accounts.collateral;
        collateral_account.amount = amount;
        collateral_account.owner = *ctx.accounts.user.key;

        // Emit event for collateral deposit
        emit!(CollateralDeposited {
            user: ctx.accounts.user.key(),
            amount,
        });

        // Transfer the collateral asset (token) to the PDA
        token::transfer(ctx.accounts.into_transfer_context(), amount)?;

        Ok(())
    }

    // Instruction to issue loan as SPL tokens based on collateral
    pub fn issue_loan(
        ctx: Context<IssueLoan>,
        loan_amount: u64,
        loan_duration: i64,
        interest_rate: u64,
    ) -> Result<()> {
        // Fetch collateral value (using a price oracle)
        let collateral_value = get_collateral_value(ctx.accounts.collateral.owner)?; // Immutable borrow first
        let collateral_account = &mut ctx.accounts.collateral; // Mutable borrow starts after
        let collateral_ratio = 150; // Require 150% collateral
        let max_loan_value = collateral_value * 100 / collateral_ratio;

        // Ensure the loan does not exceed the max value allowed by the collateral
        require!(loan_amount <= max_loan_value, LoanError::InsufficientCollateral);

        // Set the loan issue time, duration, and interest rate
        let current_timestamp = Clock::get()?.unix_timestamp;
        collateral_account.loan_issued_at = current_timestamp;
        collateral_account.loan_duration = loan_duration;
        collateral_account.loan_interest_rate = interest_rate;

        // Mint the SPL tokens to the user's token account
        token::mint_to(ctx.accounts.into_mint_to_context(), loan_amount)?;

        Ok(())
    }

    // Instruction to repay the loan and release the collateral
    pub fn repay_loan(ctx: Context<RepayLoan>, amount: u64) -> Result<()> {
        // Transfer tokens first (immutable borrow)
        token::transfer(ctx.accounts.into_repay_context(), amount)?; 
        
        let collateral_account = &mut ctx.accounts.collateral; // Mutable borrow starts here
        let current_timestamp = Clock::get()?.unix_timestamp;

        require!(collateral_account.amount > 0, LoanError::NoCollateralDeposited);

        // Calculate interest due
        let loan_duration_elapsed = current_timestamp - collateral_account.loan_issued_at;
        let interest_due = calculate_compound_interest(
            collateral_account.amount,
            collateral_account.loan_interest_rate,
            loan_duration_elapsed,
        );
        let total_repayable_amount = collateral_account.amount + interest_due;

        // Ensure sufficient amount is repaid
        require!(amount >= total_repayable_amount, LoanError::InsufficientRepayment);

        collateral_account.amount = 0; // Safe to mutate after immutable borrow is finished

        emit!(LoanRepaid {
            user: ctx.accounts.collateral.owner, // Corrected user field to use owner of collateral
            amount,
        });

        Ok(())
    }

    // Instruction to liquidate collateral if the loan isn't repaid in time
    pub fn liquidate_collateral(ctx: Context<LiquidateCollateral>) -> Result<()> {
        let amount = ctx.accounts.collateral.amount; // Immutable borrow to get amount
        token::transfer(ctx.accounts.into_liquidation_context(), amount)?; // Immutable borrow

        let collateral_account = &mut ctx.accounts.collateral; // Mutable borrow starts here
        let current_timestamp = Clock::get()?.unix_timestamp;

        let expiration_time = collateral_account.loan_issued_at + collateral_account.loan_duration;
        let grace_period = 24 * 60 * 60; // 24 hours grace period
        require!(current_timestamp > expiration_time + grace_period, LoanError::LoanNotExpired);
        
        collateral_account.amount = 0; // Safe to mutate after immutable borrow is finished

        emit!(CollateralLiquidated {
            user: ctx.accounts.collateral.owner,
            liquidator: ctx.accounts.liquidator.key(),
            amount: amount,
        });

        Ok(())
    }

    // Instruction to withdraw partial collateral
    pub fn withdraw_collateral(ctx: Context<WithdrawCollateral>, amount: u64) -> Result<()> {
        let collateral_value = get_collateral_value(ctx.accounts.collateral.owner)?; // Immutable borrow first
        let collateral_account = &mut ctx.accounts.collateral; // Mutable borrow starts after

        // Ensure remaining collateral is sufficient for the loan
        let required_collateral = (collateral_account.amount * 100) / 150;
        require!(
            collateral_value - amount >= required_collateral,
            LoanError::InsufficientCollateralRemaining
        );

        // Transfer the partial collateral back to the user
        collateral_account.amount -= amount;
        token::transfer(ctx.accounts.into_transfer_context(), amount)?;

        emit!(CollateralWithdrawn {
            user: ctx.accounts.user.key(),
            amount,
        });

        Ok(())
    }

    // Instruction to refinance the loan
    pub fn refinance_loan(
        ctx: Context<RefinanceLoan>,
        new_duration: i64,
        new_interest_rate: u64,
    ) -> Result<()> {
        let collateral_account = &mut ctx.accounts.collateral;
        collateral_account.loan_duration = new_duration;
        collateral_account.loan_interest_rate = new_interest_rate;

        emit!(LoanRefinanced {
            user: ctx.accounts.collateral.owner,
            new_duration,
            new_interest_rate,
        });

        Ok(())
    }
}

#[derive(Accounts)]
pub struct DepositCollateral<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(
        init, 
        payer = user, 
        space = 8 + 32 + 8 + 8 + 8 + 8, // Account discriminator + Pubkey (owner) + amount (u64) + timestamps + interest rate
        seeds = [b"collateral", user.key().as_ref()],
        bump
    )]
    pub collateral: Account<'info, CollateralAccount>,
    #[account(mut)]
    pub user_token_account: Account<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct IssueLoan<'info> {
    #[account(mut)]
    pub collateral: Account<'info, CollateralAccount>,
    #[account(mut)]
    pub loan_token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub loan_mint: Account<'info, Mint>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RepayLoan<'info> {
    #[account(mut)]
    pub collateral: Account<'info, CollateralAccount>,
    #[account(mut)]
    pub user_token_account: Account<'info, TokenAccount>,
    #[account(mut)]
    pub loan_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct LiquidateCollateral<'info> {
    #[account(mut)]
    pub collateral: Account<'info, CollateralAccount>,
    pub liquidator: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct WithdrawCollateral<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut)]
    pub collateral: Account<'info, CollateralAccount>,
    #[account(mut)]
    pub user_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct RefinanceLoan<'info> {
    #[account(mut)]
    pub collateral: Account<'info, CollateralAccount>,
    pub user: Signer<'info>,
}

// The struct to store collateral and loan info
#[account]
pub struct CollateralAccount {
    pub owner: Pubkey,
    pub amount: u64,            // Amount of collateral locked
    pub loan_issued_at: i64,    // Timestamp of loan issue
    pub loan_duration: i64,     // Loan duration (in seconds)
    pub loan_interest_rate: u64, // Interest rate in percentage
}

// Helper functions for token transfers
impl<'info> DepositCollateral<'info> {
    fn into_transfer_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.user_token_account.to_account_info(),
            to: self.collateral.to_account_info(),
            authority: self.user.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

impl<'info> IssueLoan<'info> {
    fn into_mint_to_context(&self) -> CpiContext<'_, '_, '_, 'info, MintTo<'info>> {
        let cpi_accounts = MintTo {
            mint: self.loan_mint.to_account_info(),
            to: self.loan_token_account.to_account_info(),
            authority: self.loan_mint.to_account_info(), // Correctly using MintTo authority
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

impl<'info> RepayLoan<'info> {
    fn into_repay_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.user_token_account.to_account_info(),
            to: self.loan_token_account.to_account_info(),
            authority: self.collateral.to_account_info(),  // Fixed authority
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

impl<'info> LiquidateCollateral<'info> {
    fn into_liquidation_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.collateral.to_account_info(),
            to: self.liquidator.to_account_info(),
            authority: self.collateral.to_account_info(), // Set collateral PDA as authority
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

impl<'info> WithdrawCollateral<'info> {
    fn into_transfer_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.collateral.to_account_info(),
            to: self.user_token_account.to_account_info(),
            authority: self.collateral.to_account_info(),  // Correct authority
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

// Placeholder function for collateral valuation (use an oracle in practice)
fn get_collateral_value(_owner: Pubkey) -> Result<u64> {
    // Fetch value from a real-time oracle (e.g., Pyth or Switchboard)
    Ok(10000) // Placeholder value
}

// Function to calculate compound interest
fn calculate_compound_interest(principal: u64, interest_rate: u64, time_elapsed: i64) -> u64 {
    let rate_decimal = interest_rate as f64 / 100.0;
    let time_in_years = time_elapsed as f64 / (365.0 * 24.0 * 3600.0);
    let compound_interest = principal as f64 * (1.0 + rate_decimal).powf(time_in_years);
    compound_interest as u64
}

// Event definitions
#[event]
pub struct CollateralDeposited {
    pub user: Pubkey,
    pub amount: u64,
}

#[event]
pub struct LoanRepaid {
    pub user: Pubkey,
    pub amount: u64,
}

#[event]
pub struct CollateralLiquidated {
    pub user: Pubkey,
    pub liquidator: Pubkey,
    pub amount: u64,
}

#[event]
pub struct CollateralWithdrawn {
    pub user: Pubkey,
    pub amount: u64,
}

#[event]
pub struct LoanRefinanced {
    pub user: Pubkey,
    pub new_duration: i64,
    pub new_interest_rate: u64,
}

// Custom errors for the loan program
#[error_code]
pub enum LoanError {
    #[msg("Insufficient collateral to issue loan")]
    InsufficientCollateral,
    #[msg("No collateral deposited")]
    NoCollateralDeposited,
    #[msg("Loan duration has not yet expired")]
    LoanNotExpired,
    #[msg("Insufficient repayment amount including interest")]
    InsufficientRepayment,
    #[msg("Insufficient collateral remaining for the loan")]
    InsufficientCollateralRemaining,
}
