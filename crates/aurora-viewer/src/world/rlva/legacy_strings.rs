//! The texts RLVa sends to the other party, word for word as the Legacy
//! Viewer sends them. Port of the strings of rlva_strings.xml
//! (indra/newview/skins/default/xui/<language>/rlva_strings.xml, originally
//! LGPL 2.1), which RlvStrings::initClass (indra/newview/rlvcommon.cpp) loads
//! for the interface language over the English file, key by key: a key a
//! translation lacks stays English. The other party reads these lines in
//! their IM window, and their scripts may compare them, so they follow the
//! Legacy Viewer's catalogue for the interface folder it would use
//! (`aurora_i18n::grid_language`), not Aurora's translations.

/// A text of rlva_strings.xml that leaves the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remote {
    /// `hidden_generic`: a teleport offer's or request's message.
    HiddenGeneric,
    /// `blocked_sendim`: the IM sent in place of one @sendim refuses.
    BlockedSendIm,
    /// `blocked_recvim_remote`: the busy reply to an IM @recvim hides.
    BlockedReceiveIm,
    /// `blocked_tplurerequest_remote`: the busy reply to a teleport offer or
    /// request a restriction refuses.
    BlockedTeleport,
    /// `imquery_list_deny`: @list or @except refused.
    ListDeny,
    /// `imquery_list_suffix`: the last line of the @list reply.
    ListSuffix,
    /// `stopim_nosession`: @stopim when @startim does not hold.
    StopImNoSession,
    /// `stopim_endsession_remote`: @stopim ended the session.
    StopImEndSession,
}

/// The text for the current interface language.
pub fn remote(key: Remote) -> &'static str {
    remote_in(aurora_i18n::grid_language(), key)
}

/// The text for a Legacy Viewer interface folder (`en`, `fr`, `zh`…).
pub fn remote_in(language: &str, key: Remote) -> &'static str {
    translated(language, key).unwrap_or_else(|| english(key))
}

fn english(key: Remote) -> &'static str {
    match key {
        Remote::HiddenGeneric => "(Hidden)",
        Remote::BlockedSendIm => "*** IM blocked by sender's viewer",
        Remote::BlockedReceiveIm => {
            "The Resident you messaged is currently prevented from reading your instant messages at the moment, please try again later."
        }
        Remote::BlockedTeleport => "The Resident is currently prevented from accepting. Please try again later.",
        Remote::ListDeny => "*** The other party respectfully requests you mind your own business (bunnies made me do it!)",
        Remote::ListSuffix => "(Use @except to see the list of active exceptions)",
        Remote::StopImNoSession => "*** The other party is not under a @startim restriction",
        Remote::StopImEndSession => "*** Session has been ended for the other party",
    }
}

/// The translated files of the Legacy Viewer; `None` where a file lacks the
/// key (the English one then applies) or has no file at all.
fn translated(language: &str, key: Remote) -> Option<&'static str> {
    use Remote::*;
    Some(match (language, key) {
        ("az", HiddenGeneric) => "(Gizli)",
        ("az", BlockedSendIm) => "*** İM göndərənin klient tərəfindən bloklanıb",
        ("az", BlockedReceiveIm) => "Mesajı göndərdiyiniz sakin hazırda ani mesajlarınızı oxuya bilmir, sonra yenidən cəhd edin.",
        ("az", BlockedTeleport) => "Hazırda sakinə qəbul etmək qadağandır. Zəhmət olmasa bir az sonra yenə cəhd edin.",
        ("az", ListDeny) => "*** Qarşı tərəf hörmətlə sizdən öz işinizlə məşğul olmağınızı xahiş edir (bunu mənə dovşanlar vadar etdi!)",
        ("az", ListSuffix) => "(Aktiv istisnaların siyahısını görmək üçün @except işarəsindən istifadə edin)",
        ("az", StopImNoSession) => "*** Qarşı tərəf @startim məhdudiyyəti altında deyil",
        ("az", StopImEndSession) => "*** Qarşı tərəf üçün sessiya bitdi",
        ("de", HiddenGeneric) => "(Versteckt)",
        ("de", BlockedSendIm) => "*** IM wurde vom Viewer des Senders blockiert.",
        ("fr", HiddenGeneric) => "(Caché)",
        ("fr", BlockedSendIm) => "*** IM bloqué par le client de l'expéditeur",
        ("fr", BlockedReceiveIm) => {
            "Le résident auquel vous avez envoyé un IM est empêché de les lire pour le moment, veuillez réessayer plus tard."
        }
        ("fr", BlockedTeleport) => "Le résident est actuellement empêché d'accepter. Veuillez réessayer plus tard.",
        ("fr", ListDeny) => {
            "*** L'autre partie vous demande respectueusement de vous occuper de vos affaires. (les lapins m'ont fait faire ça !)"
        }
        ("fr", ListSuffix) => "(Utilisez @except pour voir la liste des exceptions actives.)",
        ("fr", StopImNoSession) => "*** L'autre partie n'est pas soumise à une restriction @startim.",
        ("fr", StopImEndSession) => "*** La session a été terminée pour l'autre partie",
        ("ja", HiddenGeneric) => "（非表示）",
        ("ja", BlockedSendIm) => "※ＩＭは送信者のビューアによってブロックされています。",
        ("ja", BlockedReceiveIm) => {
            "あなたがメッセージを送信した住民は、現在インスタントメッセージを読むことができません。お手数ですが、後でもう一度お試しください。"
        }
        ("ja", ListDeny) => "※相手は、敬意を持って私を気にかけてくださいと要求しています。（バニーさんが私をそうさせたのです！）",
        // (sic: the Legacy Viewer's Japanese file names @excel)
        ("ja", ListSuffix) => "（有効な例外一覧を表示したい場合、@excelを使用します。）",
        ("ja", StopImNoSession) => "※他のパーティーは@startimによる制約を受けていません。",
        ("ja", StopImEndSession) => "※他のパーティーによってセッションは終了しました。",
        ("pl", HiddenGeneric) => "(Ukryte)",
        ("pl", BlockedSendIm) => "*** IM zablokowana przez przeglądarkę nadawcy",
        ("pl", BlockedReceiveIm) => {
            "Rezydent, do którego napisałeś/aś ma uniemożliwione czytanie wiadomości na tę chwilę, spróbuj później."
        }
        ("pl", BlockedTeleport) => {
            "Rezydent, któremu zaproponowałeś/aś teleport ma uniemożliwione odbieranie ich na tę chwilę, spróbuj później."
        }
        ("pl", ListDeny) => "*** Druga strona grzecznie prosi o zajęcie się własnymi sprawami",
        ("pl", ListSuffix) => "(użyj @except aby zobaczyć listę aktywnych wyjątków)",
        ("pl", StopImNoSession) => "*** Druga strona nie jest ograniczona za pomocą @startim",
        ("pl", StopImEndSession) => "*** Sesja została zakończona dla drugiej osoby",
        ("pt", HiddenGeneric) => "(Escondido)",
        ("pt", BlockedSendIm) => "***A mensagem instantânea foi bloqueada pelo visualizador do remetente.",
        // (sic: the first letter of the Russian file's word is a Latin C)
        ("ru", HiddenGeneric) => "(Cкрытый)",
        ("ru", BlockedSendIm) => "*** ЛС заблокировано клиентом отправителя",
        ("ru", BlockedReceiveIm) => {
            "Житель, которому вы отправили сообщение, в данный момент не может читать ваши мгновенные сообщения, повторите попытку позже."
        }
        ("ru", BlockedTeleport) => "Резиденту в настоящее время запрещено принимать. Пожалуйста, попробуйте позже.",
        ("ru", ListDeny) => "*** Другая сторона с уважением просит вас заняться своими делами (зайчики заставили меня это сделать!)",
        ("ru", ListSuffix) => "(Используйте @except, чтобы увидеть список активных исключений)",
        ("ru", StopImNoSession) => "*** Другая сторона не находится под ограничением @startim",
        ("ru", StopImEndSession) => "*** Сессия была закончена для другой стороны",
        ("zh", HiddenGeneric) => "(隱藏)",
        ("zh", BlockedSendIm) => "*** 私聊已被發送者的檢視器阻止。",
        ("zh", BlockedReceiveIm) => "您聯繫的居民目前無法閱讀您的私聊，請稍後再試。",
        ("zh", BlockedTeleport) => "該居民目前無法接受，請稍後再試。",
        ("zh", ListDeny) => "*** 對方恭敬地請求您不要過問（是兔子逼我這麼說的！）",
        ("zh", ListSuffix) => "（使用 @except 來檢視活躍的例外列表）",
        ("zh", StopImNoSession) => "*** 對方沒有受到 @startim 限制",
        ("zh", StopImEndSession) => "*** 已為對方結束會話",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlva_remote_texts_follow_the_legacy_catalogue_of_the_interface_folder() {
        assert_eq!(remote_in("en", Remote::HiddenGeneric), "(Hidden)");
        assert_eq!(remote_in("fr", Remote::HiddenGeneric), "(Caché)");
        assert_eq!(
            remote_in("ja", Remote::ListDeny),
            "※相手は、敬意を持って私を気にかけてくださいと要求しています。（バニーさんが私をそうさせたのです！）"
        );
        // a key the German file lacks, and a language with no file of its own
        assert_eq!(remote_in("de", Remote::ListDeny), english(Remote::ListDeny));
        assert_eq!(
            remote_in("de", Remote::BlockedSendIm),
            "*** IM wurde vom Viewer des Senders blockiert."
        );
        assert_eq!(
            remote_in("ko", Remote::StopImNoSession),
            "*** The other party is not under a @startim restriction"
        );
        assert_eq!(remote_in("ja", Remote::BlockedTeleport), english(Remote::BlockedTeleport));
    }
}
