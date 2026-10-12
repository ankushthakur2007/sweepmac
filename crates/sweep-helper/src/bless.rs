//! Pure, testable SMJobBless helpers (no root/certs needed).

/// Designation requirement pinning `label` to `team_id`.
pub fn requirement_string(team_id: &str, label: &str) -> String {
    format!(
        "anchor apple generic and identifier \"{label}\" and certificate leaf[subject.OU] = \"{team_id}\""
    )
}

/// True if bundled `candidate` CFBundleVersion sorts after `installed`.
/// Numeric per dot-separated component; missing trailing parts count as 0.
pub fn is_newer(candidate: &str, installed: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.split('.')
            .map(|c| c.parse::<u64>().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parts(candidate.trim()), parts(installed.trim()));
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (*a.get(i).unwrap_or(&0), *b.get(i).unwrap_or(&0));
        if x != y {
            return x > y;
        }
    }
    false
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// App Info.plist snippet: SMPrivilegedExecutables { label: requirement }.
pub fn app_privileged_executables_plist(team_id: &str, label: &str) -> String {
    let (l, r) = (xml_escape(label), xml_escape(&requirement_string(team_id, label)));
    format!("<key>SMPrivilegedExecutables</key>\n<dict>\n\t<key>{l}</key>\n\t<string>{r}</string>\n</dict>")
}

/// Helper Info.plist snippet: SMAuthorizedClients [ requirement ].
pub fn helper_authorized_clients_plist(team_id: &str, label: &str) -> String {
    let r = xml_escape(&requirement_string(team_id, label));
    format!("<key>SMAuthorizedClients</key>\n<array>\n\t<string>{r}</string>\n</array>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirement_pins_label_and_team() {
        assert_eq!(
            requirement_string("ABC123DEFG", "com.sweepmac.helper"),
            "anchor apple generic and identifier \"com.sweepmac.helper\" and certificate leaf[subject.OU] = \"ABC123DEFG\""
        );
    }

    #[test]
    fn version_compare_numeric() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.0.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0.0")); // trailing zeros equal
    }

    #[test]
    fn app_plist_has_privileged_executables() {
        let xml = app_privileged_executables_plist("TEAMID12345", "com.sweepmac.helper");
        assert!(xml.contains("<key>SMPrivilegedExecutables</key>"));
        assert!(xml.contains("<key>com.sweepmac.helper</key>"));
        assert!(xml.contains("TEAMID12345"));
        assert!(xml.contains("anchor apple generic"));
    }

    #[test]
    fn helper_plist_has_authorized_clients() {
        let xml = helper_authorized_clients_plist("TEAMID12345", "com.sweepmac.helper");
        assert!(xml.contains("<key>SMAuthorizedClients</key>"));
        assert!(xml.contains("<array>"));
        assert!(xml.contains("TEAMID12345"));
        assert!(xml.contains("identifier &quot;com.sweepmac.helper&quot;"));
    }
}
