use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOrigin(String);

impl PublicOrigin {
    pub fn parse(value: &str) -> Result<Self, OriginError> {
        Self::parse_with_http(value, false)
    }

    pub fn parse_for_development(value: &str) -> Result<Self, OriginError> {
        Self::parse_with_http(value, true)
    }

    fn parse_with_http(value: &str, allow_http: bool) -> Result<Self, OriginError> {
        let parsed = Url::parse(value).map_err(|_| OriginError::Invalid)?;
        if (parsed.scheme() != "https" && !(allow_http && parsed.scheme() == "http"))
            || parsed.host_str().is_none()
            || parsed.username() != ""
            || parsed.password().is_some()
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(OriginError::Invalid);
        }
        Ok(Self(parsed.origin().ascii_serialization()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OriginError {
    #[error("request origin is missing")]
    Missing,
    #[error("request origin is invalid")]
    Invalid,
    #[error("request origin is forbidden")]
    Forbidden,
}

pub fn validate_request_origin(
    public_origin: &PublicOrigin,
    origin: Option<&str>,
    referer: Option<&str>,
) -> Result<(), OriginError> {
    if let Some(origin) = origin {
        return (origin == public_origin.as_str())
            .then_some(())
            .ok_or(OriginError::Forbidden);
    }
    let referer = referer.ok_or(OriginError::Missing)?;
    let parsed = Url::parse(referer).map_err(|_| OriginError::Forbidden)?;
    let actual = parsed.origin().ascii_serialization();
    (actual == public_origin.as_str())
        .then_some(())
        .ok_or(OriginError::Forbidden)
}
