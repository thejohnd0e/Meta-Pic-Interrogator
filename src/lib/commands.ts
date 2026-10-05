import { invoke } from "@tauri-apps/api/core";
import type {
  DescriptionDraft,
  ImageInfo,
  InputImage,
  Preset,
  ProviderConfig,
  SaveRequest,
  VisionCapabilities,
  VisionModel,
} from "./contracts";

export const commands = {
  health: (): Promise<string> => invoke<string>("health"),
  inspectImage: (input: InputImage): Promise<ImageInfo> => invoke<ImageInfo>("inspect_image", { input }),
  describeImage: (input: InputImage, provider: ProviderConfig, preset: Preset): Promise<DescriptionDraft> =>
    invoke<DescriptionDraft>("describe_image", { input, provider, preset }),
  cancelDescription: (): Promise<void> => invoke<void>("cancel_description"),
  listPresets: (): Promise<readonly Preset[]> => invoke<readonly Preset[]>("list_presets"),
  createPreset: (preset: Preset): Promise<Preset> => invoke<Preset>("create_preset", { preset }),
  updatePreset: (preset: Preset): Promise<Preset> => invoke<Preset>("update_preset", { preset }),
  deletePreset: (presetId: string): Promise<void> => invoke<void>("delete_preset", { presetId }),
  providerStatus: (providerId: string): Promise<VisionCapabilities> => invoke<VisionCapabilities>("provider_status", { providerId }),
  refreshModels: (provider: ProviderConfig): Promise<readonly VisionModel[]> => invoke<readonly VisionModel[]>("refresh_models", { provider }),
  setCredential: (providerId: string, secret: string): Promise<void> => invoke<void>("set_credential", { providerId, secret }),
  deleteCredential: (providerId: string): Promise<void> => invoke<void>("delete_credential", { providerId }),
  savePngCopy: (request: SaveRequest): Promise<string> => invoke<string>("save_png_copy", { request }),
} as const;
