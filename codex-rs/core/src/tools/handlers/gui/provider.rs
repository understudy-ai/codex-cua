use async_trait::async_trait;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;

use super::GuiTargetRequest;
use super::ObserveState;
use super::ResolvedTarget;
use super::grounding::resolve_grounded_target;

#[async_trait]
pub(super) trait GuiGroundingProvider {
    async fn ground(
        &self,
        invocation: &ToolInvocation,
        request: GuiTargetRequest<'_>,
        capture_state: &ObserveState,
        image_bytes: &[u8],
    ) -> Result<Option<ResolvedTarget>, FunctionCallError>;
}

pub(super) struct ModelGuiGroundingProvider;

#[async_trait]
impl GuiGroundingProvider for ModelGuiGroundingProvider {
    async fn ground(
        &self,
        invocation: &ToolInvocation,
        request: GuiTargetRequest<'_>,
        capture_state: &ObserveState,
        image_bytes: &[u8],
    ) -> Result<Option<ResolvedTarget>, FunctionCallError> {
        resolve_grounded_target(invocation, request, capture_state, image_bytes).await
    }
}

pub(super) fn default_gui_grounding_provider() -> &'static ModelGuiGroundingProvider {
    static PROVIDER: ModelGuiGroundingProvider = ModelGuiGroundingProvider;
    &PROVIDER
}
