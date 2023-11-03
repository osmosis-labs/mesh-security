use std::cmp::min;

/// Tries to burn `amount` evenly from `delegations`.
/// Assigns the remainder to the first validator that has enough stake.
/// `delegations` must not be empty, or this will panic.
///
/// Returns the total amount burned, and the list of validators and amounts.
/// The total burned amount can be used to check if the user has enough stake in `delegations`.
///
/// N.B..: This can be improved by distributing the remainder evenly across validators
pub fn distribute_burn(
    delegations: &[(String, u128)],
    amount: u128,
) -> (u128, Vec<(&String, u128)>) {
    let mut burns = vec![];
    let mut burned = 0;
    let proportional_amount = amount / delegations.len() as u128;
    for (validator, delegated_amount) in delegations {
        // Check validator has `proportional_amount` delegated. Adjust accordingly if not.
        let burn_amount = min(*delegated_amount, proportional_amount);
        if burn_amount == 0 {
            continue;
        }
        burns.push((validator, burn_amount));
        burned += burn_amount;
    }
    // Adjust possible rounding issues / unfunded validators
    if burned < amount {
        // Look for the first validator that has enough stake, and burn it from there
        let burn_amount = amount - burned;
        for (validator, delegated_amount) in delegations {
            if burn_amount + proportional_amount <= *delegated_amount {
                burns.push((validator, burn_amount));
                burned += burn_amount;
                break;
            }
        }
    }
    (burned, burns)
}
