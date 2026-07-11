use crate::OpsError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    ReadOnly,
    Routine,
    Daily,
    Construction,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Routine => "routine",
            Self::Daily => "daily",
            Self::Construction => "construction",
        }
    }

    pub fn parse(value: &str) -> Result<Self, OpsError> {
        match value {
            "read_only" => Ok(Self::ReadOnly),
            "routine" => Ok(Self::Routine),
            "daily" => Ok(Self::Daily),
            "construction" => Ok(Self::Construction),
            other => Err(OpsError::Validation(format!("unknown tier: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Human,
    Ai,
}

impl TokenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Ai => "ai",
        }
    }

    pub fn parse(value: &str) -> Result<Self, OpsError> {
        match value {
            "human" => Ok(Self::Human),
            "ai" => Ok(Self::Ai),
            other => Err(OpsError::Validation(format!("unknown token kind: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    Human,
    Ai,
    LocalCli,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub actor_id: String,
    pub actor_kind: ActorKind,
    pub tier_ceiling: Tier,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_order_matches_control_plane_escalation_order() {
        assert!(Tier::ReadOnly < Tier::Routine);
        assert!(Tier::Routine < Tier::Daily);
        assert!(Tier::Daily < Tier::Construction);
    }

    #[test]
    fn tier_and_token_kind_round_trip_database_strings() {
        let cases = [
            (Tier::ReadOnly, "read_only"),
            (Tier::Routine, "routine"),
            (Tier::Daily, "daily"),
            (Tier::Construction, "construction"),
        ];
        for (tier, db) in cases {
            assert_eq!(tier.as_str(), db);
            assert_eq!(Tier::parse(db).unwrap(), tier);
        }
        assert!(Tier::parse("operator").is_err());

        assert_eq!(TokenKind::Human.as_str(), "human");
        assert_eq!(TokenKind::Ai.as_str(), "ai");
        assert_eq!(TokenKind::parse("human").unwrap(), TokenKind::Human);
        assert_eq!(TokenKind::parse("ai").unwrap(), TokenKind::Ai);
        assert!(TokenKind::parse("local_cli").is_err());
    }
}
