use crate::errors::AsldError;
use crate::model::PortForwardSpec;

pub fn validate_port_forward(spec: &PortForwardSpec) -> Result<(), AsldError> {
    if spec.listen_port == 0 || spec.guest_port == 0 {
        return Err(AsldError::InvalidArgument("port must be non-zero"));
    }
    if spec.protocol != "tcp" {
        return Err(AsldError::InvalidArgument("protocol"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use crate::model::PortForwardSpec;

    use super::validate_port_forward;

    fn rule() -> PortForwardSpec {
        PortForwardSpec {
            id: String::from("web"),
            listen_address: String::from("127.0.0.1"),
            listen_port: 3000,
            guest_port: 3000,
            protocol: String::from("tcp"),
            description: String::new(),
        }
    }

    #[test]
    fn validates_port_forward() {
        assert!(validate_port_forward(&rule()).is_ok());
    }

    #[test]
    fn rejects_zero_port() {
        let mut spec = rule();
        spec.listen_port = 0;
        assert!(validate_port_forward(&spec).is_err());
    }
}
