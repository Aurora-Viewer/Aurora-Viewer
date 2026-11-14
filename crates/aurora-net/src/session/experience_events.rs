//! LLExperienceLogDispatchHandler (llexperiencelog.cpp, LGPL 2.1): attachment
//! events for the session-local RLVa controller policy, and
//! LLEnvironmentPushDispatchHandler (llenvironment.cpp, LGPL 2.1): the
//! `PushExpEnvironment` messages of experience environment controllers.
use super::*;

/// The LLSD document of a generic message parameter.
///
/// `LLDispatcher::unpackMessage` strips one trailing NUL from the packed
/// string before `LLSDSerialize::deserialize`. The simulator writes the
/// C++ header (`<? llsd/notation ?>`), which is not the compact
/// `<?llsd/notation?>` prefix.
fn document(bytes: &[u8]) -> Option<aurora_llsd::Llsd> {
    if bytes.len() > 1024 * 1024 || bytes.is_empty() {
        return None;
    }
    let stripped = match bytes.split_last() {
        Some((0, rest)) if !rest.is_empty() => rest,
        _ => bytes,
    };
    aurora_llsd::deserialize(stripped)
        .or_else(|_| {
            if stripped.len() == bytes.len() {
                Err(aurora_llsd::LlsdError::Invalid("no trailing nul"))
            } else {
                aurora_llsd::deserialize(bytes)
            }
        })
        .ok()
}

fn environment(agent: Uuid, expected: Uuid, invoice: Uuid, parameters: &[&[u8]]) -> Option<NetEvent> {
    if expected.is_nil() || agent != expected || invoice.is_nil() {
        return None;
    }
    let message = document(parameters.first()?)?;
    matches!(
        message["action"].as_str(),
        "ClearEnvironment" | "PushFullEnvironment" | "PushPartialEnvironment"
    )
    .then_some(NetEvent::ExperienceEnvironment {
        experience: invoice,
        message,
    })
}

/// An ExperienceEvent message, its document read once: the event for the log
/// and, when an object attached something, the event for the RLVa controller
/// policy.
fn experience_event(agent: Uuid, expected: Uuid, invoice: Uuid, parameters: &[&[u8]]) -> Option<(ExperienceLogEvent, Option<NetEvent>)> {
    if expected.is_nil() || agent != expected || invoice.is_nil() {
        return None;
    }
    let document = document(parameters.first()?)?;
    let text = |index: usize| {
        parameters
            .get(index)
            .filter(|bytes| bytes.len() <= 1024)
            .map(|bytes| field_str(bytes))
            .unwrap_or_default()
    };
    let event = ExperienceLogEvent {
        experience: invoice,
        permission: document["Permission"].as_i32(),
        owner: document["OwnerID"].as_uuid(),
        object_name: text(1),
        parcel_name: text(2),
        attachment: document["IsAttachment"].as_bool(),
    };
    let attachment = (event.permission == 4 && parameters.get(1).is_some_and(|name| name.len() <= 1024) && !event.object_name.is_empty())
        .then(|| NetEvent::ExperienceAttachment {
            experience: invoice,
            object_name: event.object_name.clone(),
        });
    Some((event, attachment))
}

impl Session<'_> {
    pub(super) fn on_experience_generic(&self, agent: Uuid, method: &[u8], invoice: Uuid, parameters: &[&[u8]]) {
        match field_str(method).as_str() {
            "ExperienceEvent" => match experience_event(agent, self.agent_id(), invoice, parameters) {
                Some((event, attachment)) => {
                    emit(self.sh, NetEvent::ExperienceLog(event));
                    if let Some(attachment) = attachment {
                        emit(self.sh, attachment);
                    }
                }
                None => log::warn!(
                    "ExperienceEvent not logged: agent {agent} (ours {}), invoice {invoice}, {} parameter(s)",
                    self.agent_id(),
                    parameters.len()
                ),
            },
            "PushExpEnvironment" => match environment(agent, self.agent_id(), invoice, parameters) {
                Some(event) => emit(self.sh, event),
                None => {
                    let parsed = parameters.first().and_then(|bytes| document(bytes));
                    log::warn!(
                        "PushExpEnvironment dropped: agent {agent} (ours {}), invoice {invoice}, {} parameter(s), document {}",
                        self.agent_id(),
                        parameters.len(),
                        parsed
                            .as_ref()
                            .map(|message| message["action"].as_str())
                            .filter(|action| !action.is_empty())
                            .unwrap_or("unparsed")
                    );
                }
            },
            _ => {}
        }
    }

    pub(super) fn on_experience_eq(&self, event: &EqEvent) -> bool {
        if !matches!(event.message.as_str(), "GenericMessage" | "LargeGenericMessage") {
            return false;
        }
        let body = &event.body;
        if !matches!(
            field_str(body["MethodData"].at(0)["Method"].as_binary()).as_str(),
            "ExperienceEvent" | "PushExpEnvironment"
        ) {
            return false;
        }
        let parameters = body["ParamList"]
            .as_array()
            .iter()
            .take(3)
            .map(|row| row["Parameter"].as_binary())
            .collect::<Vec<_>>();
        self.on_experience_generic(
            body["AgentData"].at(0)["AgentID"].as_uuid(),
            body["MethodData"].at(0)["Method"].as_binary(),
            body["MethodData"].at(0)["Invoice"].as_uuid(),
            &parameters,
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(agent: Uuid, expected: Uuid, invoice: Uuid, parameters: &[&[u8]]) -> Option<NetEvent> {
        experience_event(agent, expected, invoice, parameters).and_then(|(_, attachment)| attachment)
    }

    #[test]
    fn an_experience_event_is_logged_with_its_object_and_parcel() {
        let agent = Uuid::from_u128(1);
        let experience = Uuid::from_u128(2);
        let document = b"{'Permission':i11,'OwnerID':u00000000-0000-0000-0000-000000000003,'IsAttachment':0}";
        let (event, attachment) = experience_event(agent, agent, experience, &[document, b"Door\0", b"Plaza\0"]).expect("logged");
        assert_eq!(
            (
                event.experience,
                event.permission,
                event.owner,
                event.object_name.as_str(),
                event.parcel_name.as_str()
            ),
            (experience, 11, Uuid::from_u128(3), "Door", "Plaza")
        );
        assert!(attachment.is_none());
        assert!(experience_event(Uuid::nil(), agent, experience, &[document]).is_none());
        assert!(experience_event(agent, agent, Uuid::nil(), &[document]).is_none());
    }

    #[test]
    fn experience_environment_push_keeps_its_invoice_and_rejects_foreign_or_unknown_actions() {
        let agent = Uuid::from_u128(1);
        let experience = Uuid::from_u128(2);
        let push =
            b"{'action':'PushFullEnvironment','action_data':{'asset_id':u00000000-0000-0000-0000-000000000009,'transition_time':r2}}";
        match environment(agent, agent, experience, &[push, b"Controller", b"Plaza"]) {
            Some(NetEvent::ExperienceEnvironment { experience: id, message }) => {
                assert_eq!(id, experience);
                assert_eq!(message["action"].as_str(), "PushFullEnvironment");
                assert_eq!(message["action_data"]["transition_time"].as_f32(), 2.0);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(environment(Uuid::nil(), agent, experience, &[push]).is_none());
        assert!(environment(agent, agent, Uuid::nil(), &[push]).is_none());
        assert!(environment(agent, agent, experience, &[b"{'action':'Explode'}"]).is_none());
        assert!(environment(agent, agent, experience, &[b"broken"]).is_none());
    }

    #[test]
    fn a_full_environment_push_keeps_the_serialize_header_and_the_packed_nul() {
        let agent = Uuid::from_u128(1);
        let experience = Uuid::parse_str("340191e2-a7e7-11ed-9ef6-0242ac110003").unwrap();
        let asset = Uuid::parse_str("03189ca5-3838-0912-86ad-dfb434e8ed69").unwrap();
        let mut push = b"<? llsd/notation ?>\n{'action':'PushFullEnvironment','action_data':{'asset_id':u03189ca5-3838-0912-86ad-dfb434e8ed69,'transition_time':r3},'Permission':i17,'OwnerID':u607763b1-9b52-4649-bf2f-57d2dc2f219c}".to_vec();
        push.push(0);
        match environment(
            agent,
            agent,
            experience,
            &[&push, b"Continental EEP control - LOBBY\0", b"The Continental Club\0"],
        ) {
            Some(NetEvent::ExperienceEnvironment { experience: id, message }) => {
                assert_eq!(id, experience);
                assert_eq!(message["action"].as_str(), "PushFullEnvironment");
                assert_eq!(message["action_data"]["asset_id"].as_uuid(), asset);
                assert_eq!(message["action_data"]["transition_time"].as_f32(), 3.0);
                assert_eq!(message["Permission"].as_i32(), 17);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rlva_experience_attachment_dispatch_preserves_invoice_name_and_permission() {
        let agent = Uuid::from_u128(1);
        let experience = Uuid::from_u128(2);
        for document in [
            b"{'Permission':i4}".as_slice(),
            b"<llsd><map><key>Permission</key><integer>4</integer></map></llsd>",
        ] {
            assert!(
                matches!(attachment(agent, agent, experience, &[document, b"Temporary controller\0"]), Some(NetEvent::ExperienceAttachment { experience: id, object_name }) if id == experience && object_name == "Temporary controller")
            );
            assert!(attachment(Uuid::nil(), agent, experience, &[document, b"Controller"]).is_none());
            assert!(attachment(agent, agent, experience, &[document]).is_none());
        }
        for document in [b"{'Permission':i3}".as_slice(), b"broken"] {
            assert!(attachment(agent, agent, experience, &[document, b"Controller"]).is_none());
        }
    }
}
