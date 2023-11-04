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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribute_burn_works() {
        let delegations = vec![
            ("validator1".to_string(), 100),
            ("validator2".to_string(), 200),
            ("validator3".to_string(), 300),
        ];
        let (burned, burns) = distribute_burn(&delegations, 100);
        assert_eq!(burned, 100);
        assert_eq!(burns.len(), 4);
        assert_eq!(burns[0].0, "validator1");
        assert_eq!(burns[0].1, 33);
        assert_eq!(burns[1].0, "validator2");
        assert_eq!(burns[1].1, 33);
        assert_eq!(burns[2].0, "validator3");
        assert_eq!(burns[2].1, 33);
        // And the remainder
        assert_eq!(burns[3].0, "validator1");
        assert_eq!(burns[3].1, 1);
    }

    /// Panics on empty delegations
    #[test]
    #[should_panic]
    fn distribute_burn_empty_distributions() {
        let delegations = vec![];
        distribute_burn(&delegations, 100);
    }

    #[test]
    fn distribute_burn_one_validator() {
        let delegations = vec![("validator1".to_string(), 100)];
        let (burned, burns) = distribute_burn(&delegations, 100);
        assert_eq!(burned, 100);
        assert_eq!(burns.len(), 1);
        assert_eq!(burns[0].0, "validator1");
        assert_eq!(burns[0].1, 100);
    }

    /// Some validators do not have enough funds, so the remainder is burned from the first validator
    /// that has enough funds
    #[test]
    fn distribute_burn_unfunded_validator() {
        let delegations = vec![
            ("validator1".to_string(), 100),
            ("validator2".to_string(), 1),
        ];
        let (burned, burns) = distribute_burn(&delegations, 101);
        assert_eq!(burned, 101);
        assert_eq!(burns.len(), 3);
        assert_eq!(burns[0].0, "validator1");
        assert_eq!(burns[0].1, 50);
        assert_eq!(burns[1].0, "validator2");
        assert_eq!(burns[1].1, 1);
        assert_eq!(burns[2].0, "validator1");
        assert_eq!(burns[2].1, 50);
    }

    /// There are not enough funds to burn, so the returned burned amount is less that the requested amount
    #[test]
    fn distribute_burn_insufficient_delegations() {
        let delegations = vec![
            ("validator1".to_string(), 100),
            ("validator2".to_string(), 1),
        ];
        let (burned, burns) = distribute_burn(&delegations, 102);
        assert_eq!(burned, 52);
        assert_eq!(burns.len(), 2);
        assert_eq!(burns[0].0, "validator1");
        assert_eq!(burns[0].1, 51);
        assert_eq!(burns[1].0, "validator2");
        assert_eq!(burns[1].1, 1);
    }

    /// There are enough funds to burn, but they are not consolidated enough in a single delegation.
    /// This is a limitation of the current impl.
    #[test]
    fn distribute_burn_insufficient_whole_delegation() {
        let delegations = vec![
            ("validator1".to_string(), 29),
            ("validator2".to_string(), 30),
            ("validator3".to_string(), 31),
            ("validator4".to_string(), 1),
        ];
        assert_eq!(
            delegations.iter().map(|(_, amount)| amount).sum::<u128>(),
            91
        );
        let (burned, burns) = distribute_burn(&delegations, 91);
        assert_eq!(burned, 67);
        assert_eq!(burns.len(), 4);
        assert_eq!(burns[0].0, "validator1");
        assert_eq!(burns[0].1, 22);
        assert_eq!(burns[1].0, "validator2");
        assert_eq!(burns[1].1, 22);
        assert_eq!(burns[2].0, "validator3");
        assert_eq!(burns[2].1, 22);
        assert_eq!(burns[3].0, "validator4");
        assert_eq!(burns[3].1, 1);
    }
}
