export const ERROR_CATEGORIES = [
  "authentication",
  "unavailable_model",
  "no_vision_support",
  "rate_limit",
  "network",
  "content_rejected",
  "malformed_response",
  "cancellation",
  "local_image",
  "local_metadata",
  "invalid_path",
  "empty_description",
  "not_implemented",
] as const;

export type ErrorCategory = (typeof ERROR_CATEGORIES)[number];

export type InputImage = Readonly<{ path: string }>;
export type ImageInfo = Readonly<{ path: string; format: string; width: number; height: number; hasAlpha: boolean }>;
export type Preset = Readonly<{ id: string; name: string; prompt: string }>;
export type ProviderConfig = Readonly<{ providerId: string; modelId: string; endpoint: string | null }>;
export type VisionModel = Readonly<{ id: string; displayName: string; visionCapable: boolean }>;
export type VisionCapabilities = Readonly<{ imageInput: boolean; streaming: boolean; usageReporting: boolean }>;
export type DescriptionDraft = Readonly<{ text: string; isDirty: boolean }>;
export type Provenance = Readonly<{
  schemaVersion: number;
  provider: string;
  model: string;
  presetId: string;
  presetName: string;
  presetPrompt: string;
  createdAtUtc: string;
  appVersion: string;
}>;
export type SaveRequest = Readonly<{ sourcePath: string; destinationPath: string; description: string; provenance: Provenance }>;
export type AppError = Readonly<{ category: ErrorCategory; message?: string }>;
export type SettingsDocument = Readonly<{
  schemaVersion: number;
  providerId: string | null;
  modelId: string | null;
  endpoint: string | null;
  presetId: string | null;
}>;

const isErrorCategory = (value: unknown): value is ErrorCategory =>
  typeof value === "string" && ERROR_CATEGORIES.some((category) => category === value);

export const decodeAppError = (value: unknown): AppError => {
  if (typeof value !== "object" || value === null || !("category" in value)) {
    return { category: "malformed_response", message: "Backend returned an invalid error" };
  }

  const category = value.category;
  if (!isErrorCategory(category)) {
    return { category: "malformed_response", message: "Backend returned an unknown error category" };
  }

  const message = "message" in value && typeof value.message === "string" ? value.message : undefined;
  return message === undefined ? { category } : { category, message };
};
