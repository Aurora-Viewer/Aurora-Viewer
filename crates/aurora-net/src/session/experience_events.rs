//! LLExperienceLogDispatchHandler (llexperiencelog.cpp, LGPL 2.1): attachment
//! events for the session-local RLVa controller policy, and
//! LLEnvironmentPushDispatchHandler (llenvironment.cpp, LGPL 2.1): the
//! `PushExpEnvironment` messages of experience environment controllers.
use super::*;

/// The LLSD document of a generic message parameter (notation, XML or binary,
/// as LLSDSerialize::deserialize reads it).
fn document(bytes: &[u8]) -> Option<aurora_llsd::Llsd> {
    if bytes.len() > 1024 * 1024 {
        return None;
    }
    let bytes = bytes.strip_prefix(b"<?llsd/notation?>").unwrap_or(bytes);
    aurora_llsd::from_notation(bytes)
        .or_else(|_| aurora_llsd::from_xml(bytes))
        .or_else(|_| aurora_llsd::from_binary(bytes).map(|(document, _)| document))
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

fn attachment(agent: Uuid, expected: Uuid, invoice: Uuid, parameters: &[&[u8]]) -> Option<NetEvent> {
    if expected.is_nil() || agent != expected || invoice.is_nil() {
        return None;
    }
    let document = document(parameters.first()?)?;
    if document["Permission"].as_i32() != 4 {
        return None;
    }
    let name = parameters.get(1)?;
    if name.len() > 1024 {
        return None;
    }
    let object_name = field_str(name);
    (!object_name.is_empty()).then_some(NetEvent::ExperienceAttachment {
        experience: invoice,
        object_name,
    })
}

/// The event of an ExperienceEvent message for the log.
fn log_event(agent: Uuid, expected: Uuid, invoice: Uuid, parameters: &[&[u8]]) -> Option<ExperienceLogEvent> {
    if expected.is_nil() || agent != expected || invoice.is_nil() {
        return None;
    }
    let document = parameters.first()?;
    if document.len() > 1024 * 1024 {
        return None;
    }
    let document = document.strip_prefix(b"<?llsd/notation?>").unwrap_or(document);
    let document = aurora_llsd::from_notation(document)
        .or_else(|_| aurora_llsd::from_xml(document))
        .or_else(|_| aurora_llsd::from_binary(document).map(|(document, _)| document))
        .ok()?;
    let text = |index: usize| {
        parameters
            .get(index)
            .filter(|bytes| bytes.len() <= 1024)
            .map(|bytes| field_str(bytes))
            .unwrap_or_default()
    };
    Some(ExperienceLogEvent {
        experience: invoice,
        permission: document["Permission"].as_i32(),
        owner: document["OwnerID"].as_uuid(),
        object_name: text(1),
        parcel_name: text(2),
        attachment: document["IsAttachment"].as_bool(),
    })
}

impl Session<'_> {
    pub(super) fn on_experience_generic(&self, agent: Uuid, method: &[u8], invoice: Uuid, parameters: &[&[u8]]) {
        let method = field_str(method);
        if method == "ExperienceEvent"
            && let Some(event) = log_event(agent, self.agent_id(), invoice, parameters)
        {
            emit(self.sh, NetEvent::ExperienceLog(event));
        }
        let event = match method.as_str() {
            "ExperienceEvent" => attachment(agent, self.agent_id(), invoice, parameters),
            "PushExpEnvironment" => environment(agent, self.agent_id(), invoice, parameters),
            _ => None,
        };
        if let Some(event) = event {
            emit(self.sh, event);
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
