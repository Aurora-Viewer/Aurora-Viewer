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
    let bytes = bytes.trim_ascii_start();
    // LLSDSerialize::deserialize accepts spaced and case-insensitive codec
    // headers, including the standard "<? llsd/notation ?>" serializer form.
    if let Some(header) = bytes.strip_prefix(b"<?")
        && let Some(end) = header.windows(2).position(|w| w == b"?>")
    {
        let name = header[..end].trim_ascii();
        let body = header[end + 2..].trim_ascii_start();
        if name.eq_ignore_ascii_case(b"llsd/notation") {
            return aurora_llsd::from_notation(body).ok();
        }
        if name.eq_ignore_ascii_case(b"llsd/binary") {
            return aurora_llsd::from_binary(body).ok().map(|(document, _)| document);
        }
        if name.eq_ignore_ascii_case(b"llsd/xml") {
            return aurora_llsd::from_xml(body).ok();
        }
    }
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
            "PushExpEnvironment" => {
                if let Some(event) = environment(agent, self.agent_id(), invoice, parameters) {
                    emit(self.sh, event);
                } else {
                    // Keep controller payloads and parcel/object names out of
                    // diagnostics; an invalid document must not fail silently.
                    log::warn!("PushExpEnvironment ignored: invalid recipient, experience, action or serialized document");
                }
            }
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
    fn experience_environment_push_accepts_standard_serialized_documents() {
        let agent = Uuid::from_u128(1);
        let experience = Uuid::from_u128(2);
        let message = aurora_llsd::llsd_map! {
            "action" => "PushPartialEnvironment",
            "action_data" => aurora_llsd::llsd_map! {
                "transition_time" => 0.5,
                "settings" => aurora_llsd::llsd_map! {
                    "sky" => aurora_llsd::llsd_map! { "gamma" => 2.0 },
                },
            },
        };
        let notation = aurora_llsd::to_notation(&message);
        let binary = aurora_llsd::to_binary(&message);
        let xml = aurora_llsd::to_xml(&message);
        for bytes in [
            format!("<? llsd/notation ?>\n{notation}\0").into_bytes(),
            format!("  \n<? LLSD/Notation ?>\r\n{notation}").into_bytes(),
            format!("<?llsd/notation?>\n{notation}").into_bytes(),
            notation.into_bytes(),
            xml.clone(),
            [b"<? LLSD/XML ?>\n".as_slice(), &xml].concat(),
            binary.clone(),
            [b"<?llsd/binary?>\n".as_slice(), &binary].concat(),
        ] {
            let decoded = environment(agent, agent, experience, &[&bytes]);
            assert!(
                matches!(decoded, Some(NetEvent::ExperienceEnvironment { experience: id, message: ref value }) if id == experience && value == &message),
                "serializer document was dropped"
            );
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
