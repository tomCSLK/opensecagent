//! The texts of the enrollment dialogs (Linux: `dialog`, Windows: `windialog`), in German and English, and what
//! both do with text from outside.

use crate::code;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    De,
    En,
}

#[derive(Clone, Copy)]
#[allow(dead_code, reason = "some texts are for one system only")]
pub enum Text {
    Title,
    Clipboard,
    Use,
    AnotherCode,
    Paste,
    NoCodeInClipboard,
    Enroll,
    NotACode,
    Damaged,
    Working,
    Done,
    FirstReportLater,
    OldScript,
    Other,
    // Windows
    CopyThenEnroll,
    ClipboardReady,
    NoCode,
    TryAgain,
    NeedsAdmin,
    Files,
}

/// A text in the language.
pub fn t(lang: Lang, text: Text) -> &'static str {
    let (de, en) = match text {
        Text::Title => ("Bei opensecmon anmelden", "Enroll with opensecmon"),
        Text::Clipboard => (
            "In der Zwischenablage liegt ein Anmeldecode für opensecmon unter {address}.\n\nDiesen Rechner damit anmelden?",
            "The clipboard holds an enrollment code for opensecmon at {address}.\n\nEnroll this computer with it?",
        ),
        Text::Use => ("Anmelden", "Enroll"),
        Text::AnotherCode => ("Anderen Code eingeben", "Enter another code"),
        Text::Paste => (
            "Kopieren Sie den Anmeldecode in opensecmon (er beginnt mit osa1_) und fügen Sie ihn hier ein. Ist er schon kopiert, genügt OK.\n\nNeuer Rechner: Assets → Rechner (Agent) → Rechner hinzufügen. Schon bekannter Rechner: auf seiner Seite „Neuen Code erzeugen“.",
            "Copy the enrollment code in opensecmon (it starts with osa1_) and paste it here. If it is already copied, OK is enough.\n\nNew computer: Assets → Computers (agent) → Add computer. Known computer: “Make a new code” on its page.",
        ),
        Text::NoCodeInClipboard => (
            "In der Zwischenablage liegt kein Anmeldecode. Kopieren Sie ihn in opensecmon mit dem Knopf „Kopieren“ neben dem Code und drücken Sie dann OK, oder fügen Sie ihn in das Feld ein.",
            "The clipboard holds no enrollment code. Copy it in opensecmon with the “Copy” button next to the code and press OK, or paste it into the field.",
        ),
        Text::Enroll => ("Anmelden", "Enroll"),
        Text::NotACode => (
            "Das ist kein Anmeldecode. Ein Anmeldecode beginnt mit osa1_. Kopieren Sie ihn in opensecmon mit dem Knopf neben dem Code.",
            "This is not an enrollment code. An enrollment code starts with osa1_. Copy it in opensecmon with the button next to the code.",
        ),
        Text::Damaged => (
            "Der Code ist unvollständig oder beschädigt. Kopieren Sie ihn in opensecmon noch einmal mit dem Knopf neben dem Code.",
            "The code is incomplete or damaged. Copy it again in opensecmon with the button next to the code.",
        ),
        Text::Working => (
            "Anmeldung bei opensecmon unter {address} …",
            "Enrolling with opensecmon at {address} …",
        ),
        Text::Done => (
            "Dieser Rechner ist bei opensecmon angemeldet als „{device}“.\n\nEr meldet seine installierte Software ab jetzt von selbst, etwa alle 15 Minuten. Sie müssen nichts weiter tun.",
            "This computer is enrolled with opensecmon as \"{device}\".\n\nFrom now on it reports its installed software by itself, about every 15 minutes. There is nothing else to do.",
        ),
        Text::FirstReportLater => (
            "Die erste Meldung hat noch nicht geklappt; der Rechner versucht es in einigen Minuten noch einmal.",
            "The first report did not work yet; the computer tries again in a few minutes.",
        ),
        Text::OldScript => (
            "Das frühere Skript opensecmon-agent wurde von diesem Rechner entfernt. In opensecmon steht der Rechner jetzt eventuell doppelt; den alten Eintrag können Sie dort löschen.",
            "The earlier script opensecmon-agent was removed from this computer. opensecmon may now list the computer twice; you can delete the old entry there.",
        ),
        Text::Other => (
            "Die Anmeldung hat nicht geklappt.",
            "The enrollment did not work.",
        ),
        Text::CopyThenEnroll => (
            "Kopieren Sie den Anmeldecode in opensecmon mit dem Knopf „Kopieren“ neben dem Code (er beginnt mit osa1_) und klicken Sie dann auf „Anmelden“.\n\nNeuer Rechner: Assets → Rechner (Agent) → Rechner hinzufügen. Schon bekannter Rechner: auf seiner Seite „Neuen Code erzeugen“.",
            "Copy the enrollment code in opensecmon with the “Copy” button next to the code (it starts with osa1_), then click “Enroll”.\n\nNew computer: Assets → Computers (agent) → Add computer. Known computer: “Make a new code” on its page.",
        ),
        Text::ClipboardReady => (
            "In der Zwischenablage liegt ein Anmeldecode für opensecmon unter {address}. Klicken Sie auf „Anmelden“, um diesen Rechner damit anzumelden.",
            "The clipboard holds an enrollment code for opensecmon at {address}. Click “Enroll” to enroll this computer with it.",
        ),
        Text::NoCode => (
            "In der Zwischenablage liegt noch kein Anmeldecode.",
            "The clipboard holds no enrollment code yet.",
        ),
        Text::TryAgain => ("Noch einmal versuchen", "Try again"),
        Text::NeedsAdmin => (
            "Die Anmeldung braucht Administratorrechte. Starten Sie sie über „Bei opensecmon anmelden“ im Startmenü.",
            "Enrolling needs administrator rights. Start it with “Enroll with opensecmon” in the start menu.",
        ),
        Text::Files => (
            "Die Dateien des Agenten konnten nicht geschrieben werden.",
            "The files of the agent could not be written.",
        ),
    };
    match lang {
        Lang::De => de,
        Lang::En => en,
    }
}

/// The message for a kind of error from `opensecagent enroll --for-dialog`.
pub fn failure(lang: Lang, kind: &str) -> Option<&'static str> {
    let (de, en) = match kind {
        "code_damaged" => return Some(t(lang, Text::Damaged)),
        "code_invalid" => (
            "Der Code ist abgelaufen, wurde schon benutzt oder durch einen neueren ersetzt. Erstellen Sie in opensecmon einen neuen Code und kopieren Sie ihn.",
            "The code has expired, was used already or was replaced by a newer one. Create a new code in opensecmon and copy it.",
        ),
        "network_not_allowed" => (
            "opensecmon nimmt aus diesem Netz keine Rechner an. Fragen Sie, wer opensecmon betreibt.",
            "opensecmon does not accept computers from this network. Ask whoever runs opensecmon.",
        ),
        "unreachable" => (
            "opensecmon unter {address} ist nicht erreichbar. Ist dieser Rechner im richtigen Netz (z. B. im Büro- oder Heimnetz)? Versuchen Sie es dann noch einmal.",
            "opensecmon at {address} cannot be reached. Is this computer in the right network (e.g. the office or home network)? Then try again.",
        ),
        "untrusted" => (
            "Die Gegenstelle unter {address} hat sich nicht als das opensecmon aus dem Code ausgewiesen (anderes Zertifikat). Es wurde nichts gesendet. Fragen Sie, wer opensecmon betreibt.",
            "The other side at {address} did not prove to be the opensecmon of the code (another certificate). Nothing was sent. Ask whoever runs opensecmon.",
        ),
        "wait" => (
            "opensecmon ist gerade beschäftigt. Versuchen Sie es in einer Minute noch einmal.",
            "opensecmon is busy right now. Try again in a minute.",
        ),
        "unsupported_system" => (
            "opensecmon unterstützt dieses System nicht.",
            "opensecmon does not support this system.",
        ),
        "not_authorized" => (
            "Für die Anmeldung braucht es das Passwort eines Administrators dieses Rechners.",
            "Enrolling needs the password of an administrator of this computer.",
        ),
        "no_pkexec" => (
            "pkexec fehlt; ohne pkexec kann der Dialog den Rechner nicht anmelden. Im Terminal geht es mit „sudo opensecagent enroll“.",
            "pkexec is missing; without it the dialog cannot enroll the computer. In a terminal, use \"sudo opensecagent enroll\".",
        ),
        _ => return None,
    };
    Some(match lang {
        Lang::De => de,
        Lang::En => en,
    })
}

/// The code from the clipboard and the address of its opensecmon, if the clipboard holds a valid code.
pub fn offered_code(clipboard: Option<String>) -> Option<(String, String)> {
    let text = clipboard?;
    let address = code::decode(&text).ok()?.address;
    Some((text, address))
}

/// Text from outside (server messages, names) without control characters and without `<` and `>`, which kdialog
/// would take as markup.
pub fn plain(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .filter(|c| !matches!(c, '<' | '>'))
        .take(500)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_valid_code_in_the_clipboard_is_offered() {
        let code = crate::code::tests::encode("192.0.2.10:8444", None, "ABCD-EFGH");
        assert_eq!(
            offered_code(Some(code.clone())),
            Some((code.clone(), "192.0.2.10:8444".to_owned()))
        );
        assert_eq!(offered_code(None), None);
        assert_eq!(offered_code(Some(String::new())), None);
        assert_eq!(offered_code(Some("some other text".into())), None);
        // A damaged code is not offered; the dialog then says what is wrong with it.
        let damaged = format!("{}x", &code[..code.len() - 1]);
        assert_eq!(offered_code(Some(damaged)), None);
    }

    #[test]
    fn every_error_kind_of_the_agent_has_words() {
        for lang in [Lang::De, Lang::En] {
            for kind in [
                "code_damaged",
                "code_invalid",
                "network_not_allowed",
                "unreachable",
                "untrusted",
                "wait",
                "unsupported_system",
                "not_authorized",
                "no_pkexec",
            ] {
                assert!(failure(lang, kind).is_some(), "{kind}");
            }
            assert!(failure(lang, "other").is_none());
        }
    }

    #[test]
    fn outside_text_cannot_become_markup() {
        assert_eq!(plain("<b>pc</b>\u{7}"), "bpc/b");
    }
}
