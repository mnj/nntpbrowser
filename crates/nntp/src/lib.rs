//! NNTP reader client. The protocol, compression, pool, and failover live here.
//! Article yEnc, NZB, and PAR2 do not.

mod body_budget;
mod codec;
mod compress;
mod conn;
mod connection_stats;
mod error;
mod inflate;
mod pool;
mod sasl;
mod traffic;
mod types;
mod yenc;

pub use body_budget::BodyBudget;
pub use conn::{
    ActiveTime, Article, ArticleIdentity, ArticleRange, Connection, DistributionPattern,
    GroupDescription, HeaderBytes, HeaderScope, HeaderValue, ListGroup, NewsDateTime, RangeRef,
};
pub use connection_stats::ConnectionSnapshot;
pub use error::Error;
pub use pool::{Farm, Pool};
pub use traffic::{Traffic, TrafficSnapshot};
pub use types::{
    ArticleRef, Capabilities, Config, GroupInfo, GroupStatus, HeaderStrategy, OverviewLine,
    ServerEndpoint, TlsMode,
};

#[cfg(test)]
mod tests;
