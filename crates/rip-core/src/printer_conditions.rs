//! Advisory interpretation of device-reported printer reasons, never print authority.
use serde_json::{Value, json};
pub const PROFILE: &str = "org.jrip.printer-conditions-v1";
/// Inputs come from the bounded Native IPP parser. Unknown keywords remain visible.
pub fn describe(state: i32, reasons: &[String]) -> Value {
    let mut conditions = Vec::new();
    let mut inconsistencies = Vec::new();
    let unique: std::collections::BTreeSet<_> = reasons.iter().collect();
    for raw in unique {
        if raw == "none" {
            continue;
        }
        let (base, severity, source) = if let Some(base) = raw.strip_suffix("-error") {
            (base, "error", "suffix")
        } else if let Some(base) = raw.strip_suffix("-warning") {
            (base, "warning", "suffix")
        } else if let Some(base) = raw.strip_suffix("-report") {
            (base, "report", "suffix")
        } else {
            (
                raw.as_str(),
                match state {
                    5 => "error",
                    3 | 4 => "warning",
                    _ => "unknown",
                },
                "printer-state-default",
            )
        };
        if severity == "error" && state != 5 {
            inconsistencies.push("ERROR_WITHOUT_STOPPED_STATE");
        }
        let (category, action, en, ja, zh) = match base {
            "media-empty" | "media-needed" | "media-low" => (
                "media",
                "CHECK_AND_LOAD_MEDIA",
                "Check the required media and load the appropriate tray",
                "必要な用紙とトレイを確認し、補給してください",
                "检查所需纸张并补充适当纸盒",
            ),
            "media-jam" => (
                "media",
                "CLEAR_MEDIA_JAM",
                "Follow the device instructions to clear the media jam",
                "機器の手順に従って紙詰まりを解消してください",
                "按照设备说明清除卡纸",
            ),
            "toner-low" | "toner-empty" | "marker-supply-low" | "marker-supply-empty" => (
                "supply",
                "CHECK_MARKER_SUPPLY",
                "Check and replenish the indicated toner, ink or other marking supply",
                "該当するトナー・インク等の消耗品を確認し、補給してください",
                "检查并补充相应碳粉、墨水或其他成像耗材",
            ),
            "marker-waste-almost-full" | "marker-waste-full" => (
                "supply",
                "CHECK_WASTE_RECEPTACLE",
                "Check the waste receptacle using the device instructions",
                "機器の手順に従って廃棄容器を確認してください",
                "按照设备说明检查废料容器",
            ),
            "cover-open" | "door-open" | "interlock-open" => (
                "device",
                "CHECK_COVERS",
                "Check the indicated cover, door or interlock",
                "該当するカバー・ドア・インターロックを確認してください",
                "检查相应盖板、门或联锁装置",
            ),
            "input-tray-missing" => (
                "media",
                "CHECK_INPUT_TRAY",
                "Check that the required input tray is installed",
                "必要な給紙トレイの装着を確認してください",
                "检查所需进纸盒是否已安装",
            ),
            "output-area-full" | "output-area-almost-full" | "output-tray-missing" => (
                "output",
                "CHECK_OUTPUT_AREA",
                "Check the output tray or finishing area",
                "排紙トレイ・仕上げ部を確認してください",
                "检查出纸盘或后处理区域",
            ),
            "paused" | "moving-to-paused" | "stopping" | "stopped-partly" => (
                "device",
                "REVIEW_DEVICE_PAUSE",
                "Review the device pause or stop condition with the operator",
                "管理者と機器の一時停止・停止状態を確認してください",
                "与操作员检查设备暂停或停止状态",
            ),
            "connecting-to-device" | "timed-out" => (
                "connection",
                "CHECK_DEVICE_CONNECTION",
                "Check the device connection and query its current status",
                "機器接続を確認し、現在の状態を再照会してください",
                "检查设备连接并重新查询当前状态",
            ),
            "other" => (
                "device",
                "INSPECT_DEVICE",
                "Inspect the device status and operator instructions",
                "機器の状態と操作手順を確認してください",
                "检查设备状态与操作说明",
            ),
            _ => (
                "unknown",
                "INSPECT_DEVICE",
                "Review the unrecognized device reason with the operator",
                "未対応の機器reasonを管理者と確認してください",
                "与操作员检查未识别的设备reason",
            ),
        };
        conditions.push(json!({"rawReason":raw,"reason":base,"known":category!="unknown","category":category,
            "severity":severity,"severitySource":source,"recovery":{"action":action,"advisoryOnly":true,"messages":{"en":en,"ja":ja,"zh-CN":zh}}}));
    }
    if reasons.iter().any(|r| r == "none") && !conditions.is_empty() {
        inconsistencies.push("NONE_WITH_OTHER_REASONS");
    }
    if state == 5 && conditions.iter().all(|c| c["severity"] != "error") {
        inconsistencies.push("STOPPED_WITHOUT_REPORTED_ERROR");
    }
    inconsistencies.sort_unstable();
    inconsistencies.dedup();
    json!({"schemaVersion":1,"profile":PROFILE,"source":"printer-state-reasons","printerState":state,
        "reasonEvidence":if reasons.is_empty(){"UNREPORTED"}else if reasons.iter().all(|r|r=="none"){"NO_REASONS_REPORTED"}else{"REPORTED"},
        "conditions":conditions,"inconsistencies":inconsistencies,"supplyLevelsKnown":false,"automaticResendPermitted":false,"authorizationGranted":false})
}
#[cfg(test)]
mod tests {
    use super::*;
    fn report(state: i32, reasons: &[&str]) -> Value {
        describe(
            state,
            &reasons.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
    }
    #[test]
    fn suffixes_defaults_unknown_and_none_preserve_device_evidence() {
        for (state, raw, severity) in [
            (3, "media-empty", "warning"),
            (5, "media-empty", "error"),
            (4, "toner-low-report", "report"),
            (5, "toner-empty-warning", "warning"),
            (3, "vendor-special-error", "error"),
        ] {
            let value = report(state, &[raw]);
            let c = &value["conditions"][0];
            assert_eq!(c["rawReason"], raw);
            assert_eq!(c["severity"], severity);
            assert_eq!(c["known"], !raw.starts_with("vendor"));
            for lang in ["en", "ja", "zh-CN"] {
                assert!(!c["recovery"]["messages"][lang].as_str().unwrap().is_empty());
            }
            assert_eq!(value["automaticResendPermitted"], false);
            assert_eq!(value["supplyLevelsKnown"], false);
        }
        assert_eq!(
            report(5, &["toner-low-warning"])["inconsistencies"],
            json!(["STOPPED_WITHOUT_REPORTED_ERROR"])
        );
        assert_eq!(report(3, &[])["reasonEvidence"], "UNREPORTED");
        assert_eq!(
            report(3, &["none"])["reasonEvidence"],
            "NO_REASONS_REPORTED"
        );
        let mixed = report(3, &["none", "media-jam-error", "media-jam-error"]);
        assert_eq!(mixed["conditions"].as_array().unwrap().len(), 1);
        assert_eq!(mixed["inconsistencies"].as_array().unwrap().len(), 2);
        assert_eq!(
            report(99, &["vendor"])["conditions"][0]["severity"],
            "unknown"
        );
    }
}
