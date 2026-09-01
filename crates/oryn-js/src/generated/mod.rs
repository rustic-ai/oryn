#[rustfmt::skip]
mod browser;

pub use browser::{
    GENERATED_INTERFACES, GeneratedInterface, GeneratedMember, GeneratedMemberId,
    GeneratedMemberKind, generated_interface, generated_member, generated_member_by_id,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InstallError {
    #[error("generated parent interface {parent} for {interface} is missing")]
    MissingParent {
        interface: &'static str,
        parent: &'static str,
    },
    #[error("binding installer rejected {interface}.{member}: {detail}")]
    Rejected {
        interface: &'static str,
        member: &'static str,
        detail: String,
    },
}

/// Engine adapter used by generated interface installation. The generated
/// registry defines prototypes and dispatch slots; handwritten Rust supplies
/// only the behavioral callback for each slot.
pub trait TemplateInstaller {
    fn begin_interface(&mut self, interface: &'static GeneratedInterface) -> Result<(), String>;
    fn install_member(
        &mut self,
        interface: &'static GeneratedInterface,
        member: &'static GeneratedMember,
    ) -> Result<(), String>;
    fn finish_interface(&mut self, interface: &'static GeneratedInterface) -> Result<(), String>;
}

pub fn install_generated_interfaces(
    installer: &mut impl TemplateInstaller,
) -> Result<(), InstallError> {
    let mut remaining = GENERATED_INTERFACES.iter().collect::<Vec<_>>();
    let mut installed = std::collections::BTreeSet::new();
    while !remaining.is_empty() {
        let before = remaining.len();
        let mut index = 0;
        while index < remaining.len() {
            let interface = remaining[index];
            if interface
                .parent
                .is_some_and(|parent| !installed.contains(parent))
            {
                index += 1;
                continue;
            }
            installer
                .begin_interface(interface)
                .map_err(|detail| InstallError::Rejected {
                    interface: interface.name,
                    member: "<prototype>",
                    detail,
                })?;
            for member in interface.members {
                installer
                    .install_member(interface, member)
                    .map_err(|detail| InstallError::Rejected {
                        interface: interface.name,
                        member: member.name,
                        detail,
                    })?;
            }
            installer
                .finish_interface(interface)
                .map_err(|detail| InstallError::Rejected {
                    interface: interface.name,
                    member: "<constructor>",
                    detail,
                })?;
            installed.insert(interface.name);
            remaining.remove(index);
        }
        if remaining.len() == before {
            let interface = remaining[0];
            return Err(InstallError::MissingParent {
                interface: interface.name,
                parent: interface.parent.unwrap_or("<cycle>"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Recorder(Vec<String>);

    impl TemplateInstaller for Recorder {
        fn begin_interface(
            &mut self,
            interface: &'static GeneratedInterface,
        ) -> Result<(), String> {
            self.0.push(format!("begin:{}", interface.name));
            Ok(())
        }

        fn install_member(
            &mut self,
            interface: &'static GeneratedInterface,
            member: &'static GeneratedMember,
        ) -> Result<(), String> {
            self.0
                .push(format!("member:{}.{}", interface.name, member.name));
            Ok(())
        }

        fn finish_interface(
            &mut self,
            interface: &'static GeneratedInterface,
        ) -> Result<(), String> {
            self.0.push(format!("finish:{}", interface.name));
            Ok(())
        }
    }

    #[test]
    fn generated_installation_is_complete_and_parent_before_child() {
        let mut recorder = Recorder::default();
        install_generated_interfaces(&mut recorder).unwrap();
        let node = recorder
            .0
            .iter()
            .position(|entry| entry == "finish:Node")
            .unwrap();
        let element = recorder
            .0
            .iter()
            .position(|entry| entry == "begin:Element")
            .unwrap();
        assert!(node < element);
        assert!(
            recorder
                .0
                .iter()
                .any(|entry| entry == "member:Document.createElement")
        );
    }

    #[test]
    fn generated_member_ids_are_unique_and_round_trip() {
        let mut ids = std::collections::BTreeSet::new();
        for interface in GENERATED_INTERFACES {
            for member in interface.members {
                assert!(ids.insert(member.id), "duplicate generated member id");
                let (resolved_interface, resolved_member) =
                    generated_member_by_id(member.id).expect("generated member id");
                assert_eq!(resolved_interface.name, interface.name);
                assert_eq!(resolved_member, member);
            }
        }
    }

    #[test]
    fn generated_constants_retain_their_webidl_values() {
        let member = generated_member("Range", "START_TO_START").expect("Range constant");
        assert_eq!(member.kind, GeneratedMemberKind::Constant);
        assert_eq!(member.constant_value, Some("0"));
    }
}
