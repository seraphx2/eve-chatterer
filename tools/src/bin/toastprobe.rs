//! Can EVE Chatterer send the Windows notification from
//! docs/design/windows-notification.html: a custom image, an attribution
//! line, and buttons whose clicks reach the app? And what does that need
//! from an unpackaged (not Store-installed) build?
//!
//! Sends one notification straight through WinRT (not the Tauri plugin),
//! then reports Windows' events for ~90 s: shown, dismissed (and why),
//! failed, and any button click. Appends everything to `toast-log.txt`.
//!
//!   toastprobe                 register a probe app identity (HKCU) first
//!   toastprobe --unregistered  skip that, to see what happens without one
//!   toastprobe --cleanup       remove the probe identity and exit

#[cfg(windows)]
fn main() {
    use eve_chatterer_core::winapi::ts;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use windows::core::{Interface, HSTRING, IInspectable};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{
        ToastActivatedEventArgs, ToastDismissedEventArgs, ToastFailedEventArgs, ToastNotification, ToastNotificationManager,
    };

    const AUMID: &str = "io.github.seraphx2.evechatterer.probe";
    let key = format!(r"HKCU\Software\Classes\AppUserModelId\{AUMID}");
    let args: Vec<String> = std::env::args().collect();

    let log = Arc::new(Mutex::new(std::fs::OpenOptions::new().create(true).append(true).open("toast-log.txt").expect("open toast-log.txt")));
    let say = {
        let log = log.clone();
        move |line: String| {
            println!("{line}");
            let _ = writeln!(log.lock().unwrap(), "{line}");
        }
    };

    let reg = |a: &[&str]| std::process::Command::new("reg").args(a).output().map(|o| o.status.success()).unwrap_or(false);
    if args.iter().any(|a| a == "--cleanup") {
        say(format!("{} removed probe identity: {}", ts(), reg(&["delete", &key, "/f"])));
        return;
    }

    // An icon for the identity and for the notification's logo slot: the
    // app's own icon, to test whether a custom image shows at all.
    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(r"..\app\src-tauri\icons\128x128.png");
    let icon = std::fs::canonicalize(&icon).expect("app icon");
    let icon_str = icon.display().to_string().trim_start_matches(r"\\?\").to_string();

    let registered = !args.iter().any(|a| a == "--unregistered");
    if registered {
        let ok = reg(&["add", &key, "/v", "DisplayName", "/t", "REG_SZ", "/d", "EVE Chatterer (probe)", "/f"])
            && reg(&["add", &key, "/v", "IconUri", "/t", "REG_SZ", "/d", &icon_str, "/f"]);
        say(format!("{} registered probe identity {AUMID}: {ok}", ts()));
    } else {
        say(format!("{} NOT registering an identity (--unregistered)", ts()));
    }

    let xml = format!(
        r#"<toast launch="action=open" activationType="foreground">
  <visual>
    <binding template="ToastGeneric">
      <text>Rilakss in Local</text>
      <text>Jarna, are you on for the fleet tonight? We are forming up in Jita at 20:00.</text>
      <text placement="attribution">Jarna · Mentioned you</text>
      <image placement="appLogoOverride" src="file:///{}"/>
    </binding>
  </visual>
  <actions>
    <action content="Switch to Jarna" arguments="switch=Jarna" activationType="foreground"/>
    <action content="Dismiss" arguments="dismiss" activationType="system"/>
  </actions>
</toast>"#,
        icon_str.replace('\\', "/")
    );

    let run = || -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml.as_str()))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;

        let s = say.clone();
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, a| {
            let what = a.as_ref().and_then(|a| a.cast::<ToastActivatedEventArgs>().ok()).and_then(|a| a.Arguments().ok()).map(|h| h.to_string());
            s(format!("{} ACTIVATED (click reached the app), arguments = {:?}", ts(), what));
            Ok(())
        }))?;
        let s = say.clone();
        toast.Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(move |_, a| {
            let why = a.as_ref().and_then(|a| a.Reason().ok()).map(|r| match r.0 {
                0 => "UserCanceled (closed it)",
                1 => "ApplicationHidden",
                2 => "TimedOut (went to Action Center)",
                _ => "other",
            });
            s(format!("{} DISMISSED: {:?}", ts(), why));
            Ok(())
        }))?;
        let s = say.clone();
        toast.Failed(&TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, a| {
            let e = a.as_ref().and_then(|a| a.ErrorCode().ok()).map(|c| format!("{c:?}"));
            s(format!("{} FAILED: {:?}", ts(), e));
            Ok(())
        }))?;

        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
        let setting = notifier.Setting().map(|v| match v.0 {
            0 => "Enabled",
            1 => "DisabledForApplication",
            2 => "DisabledForUser",
            3 => "DisabledByGroupPolicy",
            4 => "DisabledByManifest",
            _ => "unknown",
        });
        say(format!("{} notifier setting for this identity: {setting:?}", ts()));
        notifier.Show(&toast)?;
        say(format!("{} Show() returned OK; watching for 90 s: click the body, a button, or close it", ts()));
        std::thread::sleep(Duration::from_secs(90));
        Ok(())
    };
    if let Err(e) = run() {
        say(format!("{} error: {e}", ts()));
    }
    say(format!("{} done", ts()));
}

#[cfg(not(windows))]
fn main() {
    eprintln!("toastprobe is Windows only");
}
