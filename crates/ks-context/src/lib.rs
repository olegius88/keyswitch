//! The context layer of the engine, ported from the Python with bit-exact parity: the field context, the contextual
//! action model and its features, the prefix models, the policy that decides with them and the typo corrector.
//!
//! Each module is the Python module of the same name. The models read one resources folder given at load time; nothing
//! here derives a path from the executable. The two big tables, the feature weights and the term counts, are held in flat
//! arrays (`flat_table`).

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![forbid(unsafe_code)]
#![expect(clippy::disallowed_types, clippy::disallowed_methods, reason = "temporary: this crate still handles its JSON as untyped trees; step 2b of the strict-json plan types them and removes this")]
#![cfg_attr(test, expect(clippy::disallowed_macros, reason = "temporary: the tests of this crate still build their JSON as untyped trees; step 2b of the strict-json plan types them and removes this"))]

pub mod constants;
pub mod context_action_features;
pub mod context_model;
pub mod flat_table;
pub mod input_context;
pub mod prefix_model;
pub mod prefix_schema;
pub mod context_policy;
pub mod typo_correction;

pub use context_action_features::{extract_action_features, FeatureError, FeatureMap};
pub use context_model::{AfterOrigin, BeforeOrigin, ContextAction, ContextEvidence, ContextModel, ContextPrediction, TermTables};
pub use context_policy::{ContextPolicy, ContextResult, DecideOptions, PolicyError};
pub use input_context::{FieldContext, FieldReader, FieldRole, InputContext, KeyEvent};
pub use prefix_model::{PrefixInput, PrefixModel, PrefixSide};
pub use prefix_schema::VersionedPrefixModel;
pub use typo_correction::{KeyTemplates, TypoCorrector, TypoJudgement};
