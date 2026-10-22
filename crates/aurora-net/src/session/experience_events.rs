//! LLExperienceLogDispatchHandler (llexperiencelog.cpp, LGPL 2.1).
//! Only attachment events are needed by the session-local RLVa controller policy.
use super::*;

fn attachment(agent: Uuid, expected: Uuid, invoice: Uuid, parameters: &[&[u8]]) -> Option<NetEvent> {
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

impl Session<'_> {
    pub(super) fn on_experience_generic(&self, agent: Uuid, method: &[u8], invoice: Uuid, parameters: &[&[u8]]) {
        if field_str(method) == "ExperienceEvent"
            && let Some(event) = attachment(agent, self.agent_id(), invoice, parameters)
        {
            emit(self.sh, event);
        }
    }

    pub(super) fn on_experience_eq(&self, event: &EqEvent) -> bool {
        if !matches!(event.message.as_str(), "GenericMessage" | "LargeGenericMessage") {
            return false;
        }
        let body = &event.body;
        if field_str(body["MethodData"].at(0)["Method"].as_binary()) != "ExperienceEvent" {
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
