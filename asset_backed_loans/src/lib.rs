use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};  
use solana_program::sysvar::clock::Clock;
// use pyth_client::{Price, PriceFeed}; // Pyth commented out

declare_id!("GRbcvLa6oYsjh28iSkLaFff1pkjdtAc9ybNRronynnyc");

#[program]
pub mod asset_backed_loans {
    use super::*;

    // Instruction to deposit collateral into a PDA account
    pub fn deposit_collateral(ctx: Context<DepositCollateral>, amount: u64, collateral_type: u8) -> Result<()> {
        let collateral_account = &mut ctx.accounts.collateral;
        collateral_account.amount = amount;
        collateral_account.owner = *ctx.accounts.user.key;
        collateral_account.collateral_type = collateral_type;

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
        interest_type: u8,  // 0 for fixed, 1 for variable
        grace_period: i64,
    ) -> Result<()> {
        let collateral_account = &mut ctx.accounts.collateral;

        // Fetch real-time price from Pyth oracle
        // let collateral_value = get_collateral_value(&ctx.accounts.price_feed)?;  // Commented out Pyth
        let collateral_value = 10000; // Placeholder value in absence of Pyth
        let collateral_ratio = get_ltv(collateral_account.collateral_type); // LTV based on collateral type
        let max_loan_value = collateral_value * 100 / collateral_ratio;

        // Ensure the loan does not exceed the max value allowed by the collateral
        require!(loan_amount <= max_loan_value, LoanError::InsufficientCollateral);

        // Check if a loan is already active
        let current_timestamp = Clock::get()?.unix_timestamp;
        require!(
            collateral_account.loan_issued_at == 0 || 
            current_timestamp >= collateral_account.loan_issued_at + collateral_account.loan_duration + collateral_account.grace_period,
            LoanError::LoanAlreadyIssued
        );

        // Set loan terms
        collateral_account.loan_issued_at = current_timestamp;
        collateral_account.loan_duration = loan_duration;
        collateral_account.loan_interest_rate = interest_rate;
        collateral_account.interest_type = interest_type;
        collateral_account.grace_period = grace_period;

        // Mint the SPL tokens to the user's token account
        token::mint_to(ctx.accounts.into_mint_to_context(), loan_amount)?;

        // Transfer protocol fee
        let protocol_fee = loan_amount * 1 / 100;
        token::transfer(ctx.accounts.into_fee_context(), protocol_fee)?;

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
        user: ctx.accounts.collateral.owner,
        amount,
    });

    Ok(())
}

    // Instruction to liquidate collateral if the loan isn't repaid in time or health factor is low
pub fn liquidate_collateral(ctx: Context<LiquidateCollateral>) -> Result<()> {
    let reward_amount = ctx.accounts.collateral.amount * 5 / 100;
    let amount_to_liquidate = ctx.accounts.collateral.amount - reward_amount;

    // Transfer reward to liquidator first (immutable borrow)
    token::transfer(ctx.accounts.into_liquidation_context(), reward_amount)?;

    // Transfer remaining collateral to liquidator (immutable borrow again)
    token::transfer(ctx.accounts.into_liquidation_context(), amount_to_liquidate)?;

    let collateral_account = &mut ctx.accounts.collateral; // Mutable borrow starts after the immutable borrows
    let current_timestamp = Clock::get()?.unix_timestamp;

    let expiration_time = collateral_account.loan_issued_at + collateral_account.loan_duration;
    let liquidation_threshold = collateral_account.amount * 120 / 100;

    // Liquidation trigger: Either loan expiration or health factor < 1.2
    require!(
        collateral_account.amount < liquidation_threshold ||
        current_timestamp > expiration_time + collateral_account.grace_period,
        LoanError::LoanNotExpiredOrCollateralUnderwater
    );

    collateral_account.amount = 0; // Safe to mutate after immutable borrows are finished

    emit!(CollateralLiquidated {
        user: ctx.accounts.collateral.owner,
        liquidator: ctx.accounts.liquidator.key(),
        amount: amount_to_liquidate,
    });

    Ok(())
}

    // Instruction to withdraw partial collateral
    pub fn withdraw_collateral(ctx: Context<WithdrawCollateral>, amount: u64) -> Result<()> {
        // let collateral_value = get_collateral_value(&ctx.accounts.price_feed)?;  // Commented out Pyth
        let collateral_value = 10000; // Placeholder value in absence of Pyth
        let collateral_account = &mut ctx.accounts.collateral;

        // Ensure remaining collateral is sufficient for the loan
        let required_collateral = (collateral_account.amount * 100) / get_ltv(collateral_account.collateral_type);
        require!(
            collateral_value - amount >= required_collateral,
            LoanError::InsufficientCollateralRemaining
        );

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
    new_duration: Option<i64>,  // Optional argument
    new_interest_rate: Option<u64>,  // Optional argument
) -> Result<()> {
    // Extract `collateral.owner` immutably before any mutable borrow
    let owner = ctx.accounts.collateral.owner;

    let collateral_account = &mut ctx.accounts.collateral; // Mutable borrow starts here

    // Update loan parameters if provided
    if let Some(duration) = new_duration {
        collateral_account.loan_duration = duration;
    }
    if let Some(rate) = new_interest_rate {
        collateral_account.loan_interest_rate = rate;
    }

    emit!(LoanRefinanced {
        user: owner, // Use `owner` variable here
        new_duration: collateral_account.loan_duration,
        new_interest_rate: collateral_account.loan_interest_rate,
    });

    Ok(())
}
}

// Accounts and Structs

#[derive(Accounts)]
pub struct DepositCollateral<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(
        init, 
        payer = user, 
        space = 8 + 32 + 8 + 8 + 8 + 8 + 8 + 8 + 8, // Account discriminator + Pubkey (owner) + amount + timestamps + interest rate + grace period + collateral type
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
    // #[account(address = PYTH_PRICE_FEED)]  // Commented out Pyth
    // pub price_feed: AccountInfo<'info>,     // Commented out Pyth
    #[account(mut)]
    pub treasury: Account<'info, TokenAccount>, // Protocol's treasury for fees
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

// Collateral Account Struct
#[account]
pub struct CollateralAccount {
    pub owner: Pubkey,
    pub amount: u64,            // Amount of collateral locked
    pub loan_issued_at: i64,    // Timestamp of loan issue
    pub loan_duration: i64,     // Loan duration (in seconds)
    pub loan_interest_rate: u64, // Interest rate in percentage
    pub interest_type: u8,      // 0 for fixed, 1 for variable
    pub grace_period: i64,      // Grace period for the loan (in seconds)
    pub collateral_type: u8,    // 0 for gold, 1 for crypto, etc.
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
            authority: self.loan_mint.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }

    fn into_fee_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.loan_token_account.to_account_info(),
            to: self.treasury.to_account_info(),
            authority: self.loan_mint.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

impl<'info> RepayLoan<'info> {
    fn into_repay_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.user_token_account.to_account_info(),
            to: self.loan_token_account.to_account_info(),
            authority: self.collateral.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

impl<'info> LiquidateCollateral<'info> {
    fn into_liquidation_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.collateral.to_account_info(),
            to: self.liquidator.to_account_info(),
            authority: self.collateral.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

impl<'info> WithdrawCollateral<'info> {
    fn into_transfer_context(&self) -> CpiContext<'_, '_, '_, 'info, Transfer<'info>> {
        let cpi_accounts = Transfer {
            from: self.collateral.to_account_info(),
            to: self.user_token_account.to_account_info(),
            authority: self.collateral.to_account_info(),
        };
        CpiContext::new(self.token_program.to_account_info(), cpi_accounts)
    }
}

// // Pyth Oracle Fetching (commented out)
// fn get_collateral_value(price_feed: &AccountInfo) -> Result<u64> {
//     let price_feed_data = &price_feed.try_borrow_data()?;
//     let price_feed: PriceFeed = pyth_client::cast(price_feed_data);

//     if let Some(price) = price_feed.get_current_price() {
//         Ok(price.price as u64) // Return the price (after adjusting units)
//     } else {
//         Err(LoanError::OraclePriceUnavailable.into())
//     }
// }

// Function to calculate compound interest
fn calculate_compound_interest(principal: u64, interest_rate: u64, time_elapsed: i64) -> u64 {
    let rate_decimal = interest_rate as f64 / 100.0;
    let time_in_years = time_elapsed as f64 / (365.0 * 24.0 * 3600.0);
    let compound_interest = principal as f64 * (1.0 + rate_decimal).powf(time_in_years);
    compound_interest as u64
}

// Function to calculate Loan-to-Value (LTV) based on collateral type
fn get_ltv(collateral_type: u8) -> u64 {
    match collateral_type {
        0 => 70,  // Gold: 70% LTV
        1 => 50,  // Crypto: 50% LTV
        _ => 50,  // Default LTV
    }
}

// Event Definitions
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

// Custom Errors
#[error_code]
pub enum LoanError {
    #[msg("Insufficient collateral to issue loan")]
    InsufficientCollateral,
    #[msg("No collateral deposited")]
    NoCollateralDeposited,
    #[msg("Loan duration has not yet expired or collateral is underwater")]
    LoanNotExpiredOrCollateralUnderwater,
    #[msg("Insufficient repayment amount including interest")]
    InsufficientRepayment,
    #[msg("Insufficient collateral remaining for the loan")]
    InsufficientCollateralRemaining,
    #[msg("Loan already issued")]
    LoanAlreadyIssued,
    #[msg("Overpayment not allowed")]
    OverpaymentNotAllowed,
    #[msg("Oracle price unavailable")]
    OraclePriceUnavailable,
}
