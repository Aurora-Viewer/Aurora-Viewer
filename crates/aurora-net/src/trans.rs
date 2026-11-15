//! The transaction types of a money message (MoneyBalanceReply's
//! TransactionInfo, a group's account details): the values of
//! lltransactiontypes.h (indra/llinventory/lltransactiontypes.h, originally
//! LGPL 2.1) that the viewer names.

pub const GROUP_CREATE: i32 = 1002;
pub const GROUP_JOIN: i32 = 1004;
pub const UPLOAD_CHARGE: i32 = 1101;
pub const CLASSIFIED_CHARGE: i32 = 1103;
pub const OBJECT_SALE: i32 = 5000;
pub const GIFT: i32 = 5001;
pub const LAND_SALE: i32 = 5002;
pub const LAND_PASS_SALE: i32 = 5006;
pub const PAY_OBJECT: i32 = 5008;
pub const OBJECT_PAYS: i32 = 5009;
pub const GROUP_LAND_DEED: i32 = 6001;
pub const EVENT_FEE: i32 = 9003;
pub const EVENT_PRIZE: i32 = 9004;
