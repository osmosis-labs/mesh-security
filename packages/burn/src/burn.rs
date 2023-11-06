use std::cmp::min;
use std::collections::HashMap;

/// Tries to burn `amount` evenly from `delegations`.
/// Assigns the remainder to the first validator that has enough stake.
/// `delegations` must not be empty, or this will panic.
///
/// Returns the total amount burned, and the list of validators and amounts.
/// The total burned amount can be used to check if the user has enough stake in `delegations`.
///
/// N.B..: This can be improved by distributing the remainder evenly across validators.
pub fn distribute_burn(
    delegations: &[(String, u128)],
    amount: u128,
) -> (u128, Vec<(&String, u128)>) {
    let mut burns = HashMap::new();
    let mut burned = 0;
    let proportional_amount = amount / delegations.len() as u128;
    for (validator, delegated_amount) in delegations {
        // Check validator has `proportional_amount` delegated. Adjust accordingly if not.
        let burn_amount = min(*delegated_amount, proportional_amount);
        if burn_amount == 0 {
            continue;
        }
        burns
            .entry(validator)
            .and_modify(|amount| *amount += burn_amount)
            .or_insert(burn_amount);
        burned += burn_amount;
    }
    // Adjust possible rounding issues / unfunded validators
    if burned < amount {
        // Look for the first validator that has enough stake, and burn it from there
        let burn_amount = amount - burned;
        for (validator, delegated_amount) in delegations {
            if burn_amount + burns.get(&validator).unwrap_or(&0) <= *delegated_amount {
                burns
                    .entry(validator)
                    .and_modify(|amount| *amount += burn_amount)
                    .or_insert(burn_amount);
                burned += burn_amount;
                break;
            }
        }
    }
    (burned, burns.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn assert_burns(burns: &[(&String, u128)], expected: &[(&str, u128)]) {
        let mut burns = burns
            .iter()
            .map(|(validator, amount)| (validator.to_string(), *amount))
            .collect::<Vec<_>>();
        burns.sort_by(|(v1, _), (v2, _)| v1.cmp(v2));
        let expected = expected
            .iter()
            .map(|(validator, amount)| (validator.to_string(), *amount))
            .collect::<Vec<_>>();
        assert_eq!(burns, expected);
    }

    #[test]
    fn distribute_burn_works() {
        let delegations = vec![
            ("validator1".to_string(), 100),
            ("validator2".to_string(), 200),
            ("validator3".to_string(), 300),
        ];
        let (burned, burns) = distribute_burn(&delegations, 100);
        assert_eq!(burned, 100);
        assert_burns(
            &burns,
            &[("validator1", 34), ("validator2", 33), ("validator3", 33)],
        );
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
        assert_burns(&burns, &[("validator1", 100)]);
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
        assert_burns(&burns, &[("validator1", 100), ("validator2", 1)]);
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
        assert_burns(&burns, &[("validator1", 51), ("validator2", 1)]);
    }

    /// There are enough funds to burn, but they are not consolidated enough in a single delegation.
    // FIXME? This is a limitation of the current impl.
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
        assert_burns(
            &burns,
            &[
                ("validator1", 22),
                ("validator2", 22),
                ("validator3", 22),
                ("validator4", 1),
            ],
        );
    }
}
