use std::collections::BTreeSet;

use super::AllowedOutcome;

pub(super) const SCENARIOS: &[&str] = &[
    "D000-1table1column0rows",
    "D001-1table1column1row",
    "D002-1table2columns1row",
    "D003-1table3columns1row",
    "D004-1table2columns1row",
    "D005-1table3columns3rows2duplicates",
    "D006-1table1primarykey1column1row",
    "D007-1table1primarykey2columns1row",
    "D008-1table1compositeprimarykey3columns1row",
    "D009-2tables1primarykey1foreignkey",
    "D010-1table1primarykey3colums3rows",
    "D011-M2MRelations",
    "D012-2tables2duplicates0nulls",
    "D013-1table1primarykey3columns2rows1nullvalue",
    "D014-3tables1primarykey1foreignkey",
    "D015-1table3columns1composityeprimarykey3rows2languages",
    "D016-1table1primarykey10columns3rowsSQLdatatypes",
    "D017-I18NnoSpecialChars",
    "D018-1table1primarykey2columns3rows",
    "D019-1table1primarykey3columns3rows",
    "D020-1table1column5rows",
    "D021-2tables2primarykeys1foreignkeyReferencesAllNulls",
    "D022-2tables1primarykey1foreignkeyReferencesNoPrimaryKey",
    "D023-2tables2primarykeys2foreignkeysReferencesToNon-primarykeys",
    "D024-2tables2primarykeys1foreignkeyToARowWithSomeNulls",
    "D025-3tables3primarykeys3foreignkeys",
];

const R2RML_SUFFIXES: &str = "0000 0001a 0001b 0002a 0002b 0002c 0002d 0002e 0002f 0002g 0002h 0002i 0002j 0003a 0003b 0003c 0004a 0004b 0005a 0005b 0006a 0007a 0007b 0007c 0007d 0007e 0007f 0007g 0007h 0008a 0008b 0008c 0009a 0009b 0009c 0009d 0010a 0010b 0010c 0011a 0011b 0012a 0012b 0012c 0012d 0012e 0013a 0014a 0014b 0014c 0014d 0015a 0015b 0016a 0016b 0016c 0016d 0016e 0018a 0019a 0019b 0020a 0020b";
const DIRECT_SUFFIXES: &str = "0000 0001 0002 0003 0004 0005 0006 0007 0008 0009 0010 0011 0012 0013 0014 0015 0016 0017 0018 0021 0022 0023 0024 0025";
const ERROR_IDS: &str = "R2RMLTC0002c R2RMLTC0002e R2RMLTC0002f R2RMLTC0002g R2RMLTC0002h R2RMLTC0003a R2RMLTC0004b R2RMLTC0007h R2RMLTC0012c R2RMLTC0012d R2RMLTC0015b R2RMLTC0019b R2RMLTC0020b";

pub(super) fn allowed_outcomes(id: &str) -> (AllowedOutcome, AllowedOutcome) {
    let sqlite = if id == "R2RMLTC0002f" {
        AllowedOutcome::Deviation
    } else if matches!(
        id,
        "DirectGraphTC0021"
            | "DirectGraphTC0022"
            | "DirectGraphTC0023"
            | "DirectGraphTC0024"
            | "DirectGraphTC0025"
    ) {
        AllowedOutcome::Skip
    } else {
        AllowedOutcome::Pass
    };
    let postgres = if id == "R2RMLTC0002f" {
        AllowedOutcome::Deviation
    } else if matches!(
        id,
        "R2RMLTC0016a"
            | "R2RMLTC0016b"
            | "R2RMLTC0016c"
            | "R2RMLTC0016d"
            | "R2RMLTC0016e"
            | "DirectGraphTC0016"
    ) {
        AllowedOutcome::Skip
    } else {
        AllowedOutcome::Pass
    };
    (sqlite, postgres)
}

pub(super) fn expected_scenarios() -> BTreeSet<String> {
    SCENARIOS.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn expected_ids() -> BTreeSet<String> {
    prefixed_set("R2RMLTC", R2RML_SUFFIXES)
        .into_iter()
        .chain(prefixed_set("DirectGraphTC", DIRECT_SUFFIXES))
        .collect()
}

pub(super) fn expected_error_ids() -> BTreeSet<String> {
    prefixed_set("", ERROR_IDS)
}

fn prefixed_set(prefix: &str, values: &str) -> BTreeSet<String> {
    values
        .split_whitespace()
        .map(|value| format!("{prefix}{value}"))
        .collect()
}
