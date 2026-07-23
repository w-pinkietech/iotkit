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
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub actor_id: String,
    pub actor_kind: ActorKind,
    pub tier_ceiling: Tier,
}

#[cfg(test)]
#[path = "../tests/unit/tier_tests.rs"]
mod tests;
