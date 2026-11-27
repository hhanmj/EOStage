export type AoiBindingCompletionDecision = {
  closeAoiTools: boolean;
  clearAdminSearch: boolean;
};

export function completeAoiBindingDecision({
  succeeded,
  capturedAdminSearchGeneration,
  currentAdminSearchGeneration,
}: {
  succeeded: boolean;
  capturedAdminSearchGeneration: number;
  currentAdminSearchGeneration: number;
}): AoiBindingCompletionDecision {
  if (!succeeded) {
    return { closeAoiTools: false, clearAdminSearch: false };
  }
  return {
    closeAoiTools: false,
    clearAdminSearch: capturedAdminSearchGeneration === currentAdminSearchGeneration,
  };
}
