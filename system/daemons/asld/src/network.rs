use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::errors::AsldError;
use crate::model::{NetworkPolicy, NetworkValidation, PortForwardSpec};

pub fn validate_network_policy(policy: &NetworkPolicy) -> Result<(), AsldError> {
    if policy.mode != "nat" {
        return Err(AsldError::InvalidArgument("network.mode"));
    }
    if policy.dns_mode != "host-broker" {
        return Err(AsldError::InvalidArgument("network.dns_mode"));
    }
    Ok(())
}

pub fn validate_port_forward(spec: &PortForwardSpec) -> Result<(), AsldError> {
    if !valid_id(&spec.id) {
        return Err(AsldError::InvalidArgument("rule_id"));
    }
    if !valid_listen_address(&spec.listen_address) {
        return Err(AsldError::InvalidArgument("listen_address"));
    }
    if spec.listen_port == 0 || spec.guest_port == 0 {
        return Err(AsldError::InvalidArgument("port must be non-zero"));
    }
    if spec.protocol != "tcp" {
        return Err(AsldError::InvalidArgument("protocol"));
    }
    Ok(())
}

pub fn validate_network_set(
    policy: &NetworkPolicy,
    rules: &[PortForwardSpec],
) -> Vec<NetworkValidation> {
    let mut out = Vec::new();
    match validate_network_policy(policy) {
        Ok(()) => out.push(NetworkValidation {
            id: String::from("policy"),
            listen: String::new(),
            valid: true,
            exposure: String::from("nat"),
            message: format!(
                "nat enabled; dns={}; outbound={}",
                policy.dns_mode,
                if policy.allow_outbound {
                    "true"
                } else {
                    "false"
                }
            ),
        }),
        Err(err) => out.push(NetworkValidation {
            id: String::from("policy"),
            listen: String::new(),
            valid: false,
            exposure: String::from("invalid"),
            message: err.message(),
        }),
    }

    for (index, rule) in rules.iter().enumerate() {
        let mut valid = validate_port_forward(rule).is_ok();
        let mut message = if valid {
            String::from("port forward valid")
        } else {
            String::from("invalid port forward")
        };
        for previous in &rules[..index] {
            if same_listener(previous, rule) {
                valid = false;
                message = format!("listener conflicts with {}", previous.id);
                break;
            }
        }
        out.push(NetworkValidation {
            id: rule.id.clone(),
            listen: format!("{}:{}", rule.listen_address, rule.listen_port),
            valid,
            exposure: port_exposure(rule),
            message,
        });
    }
    out
}

pub fn port_exposure(rule: &PortForwardSpec) -> String {
    if rule.listen_address == "127.0.0.1"
        || rule.listen_address == "localhost"
        || rule.listen_address == "::1"
    {
        String::from("local")
    } else if rule.listen_address == "0.0.0.0" || rule.listen_address == "*" {
        String::from("public")
    } else {
        String::from("host")
    }
}

pub fn same_listener(left: &PortForwardSpec, right: &PortForwardSpec) -> bool {
    if left.protocol != right.protocol || left.listen_port != right.listen_port {
        return false;
    }
    let left_address = normalize_listen_address(&left.listen_address);
    let right_address = normalize_listen_address(&right.listen_address);
    left_address == right_address
        || (left_address == "0.0.0.0" && valid_ipv4(right_address))
        || (right_address == "0.0.0.0" && valid_ipv4(left_address))
}

fn normalize_listen_address(address: &str) -> &str {
    match address {
        "localhost" => "127.0.0.1",
        "*" => "0.0.0.0",
        other => other,
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_'))
}

fn valid_listen_address(address: &str) -> bool {
    matches!(address, "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "*") || valid_ipv4(address)
}

fn valid_ipv4(address: &str) -> bool {
    let mut parts = 0usize;
    for part in address.split('.') {
        parts += 1;
        if parts > 4 || part.is_empty() || part.len() > 3 {
            return false;
        }
        let mut value = 0u16;
        for b in part.bytes() {
            if !b.is_ascii_digit() {
                return false;
            }
            value = match value
                .checked_mul(10)
                .and_then(|v| v.checked_add((b - b'0') as u16))
            {
                Some(value) => value,
                None => return false,
            };
        }
        if value > 255 {
            return false;
        }
    }
    parts == 4
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use crate::model::{NetworkPolicy, PortForwardSpec};

    use super::{
        port_exposure, validate_network_policy, validate_network_set, validate_port_forward,
    };

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
    fn validates_nat_policy() {
        assert!(validate_network_policy(&NetworkPolicy::default()).is_ok());
        let mut policy = NetworkPolicy::default();
        policy.mode = String::from("bridge");
        assert!(validate_network_policy(&policy).is_err());
    }

    #[test]
    fn rejects_zero_port() {
        let mut spec = rule();
        spec.listen_port = 0;
        assert!(validate_port_forward(&spec).is_err());
    }

    #[test]
    fn classifies_port_exposure() {
        assert_eq!(port_exposure(&rule()), "local");
        let mut public = rule();
        public.listen_address = String::from("0.0.0.0");
        assert_eq!(port_exposure(&public), "public");
    }

    #[test]
    fn validation_reports_conflicting_listeners() {
        let first = rule();
        let mut second = rule();
        second.id = String::from("api");
        let report = validate_network_set(&NetworkPolicy::default(), &[first, second]);
        assert_eq!(report.len(), 3);
        assert!(!report[2].valid);
    }

    #[test]
    fn wildcard_listener_conflicts_with_specific_ipv4() {
        let mut first = rule();
        first.listen_address = String::from("0.0.0.0");
        let mut second = rule();
        second.id = String::from("api");
        let report = validate_network_set(&NetworkPolicy::default(), &[first, second]);
        assert_eq!(report[2].message, "listener conflicts with web");
    }
}
