# Meta Pic Interrogator Design System

## 0. Research Log
- Embedded refs: shortlisted minimalist, taste, and soft directions; picked minimalist + warm editorial treatment because Meta Pic Interrogator is an operational desktop tool.
- Lazyweb and Imagen lanes: skipped; no network/reference image is required for this native utility surface.

## 1. Atmosphere & Identity
Quiet editorial workbench: warm bone paper, charcoal ink, and one muted ochre action color. The signature is a thin image frame that turns into a provenance record, not a generic dashboard card.

## 2. Color
| Token | Value | Usage |
|---|---|---|
| --canvas | #F7F6F3 | app background |
| --surface | #FFFFFF | panels and controls |
| --ink | #25231F | primary text |
| --muted | #78736A | secondary text |
| --line | #E4E0D8 | dividers |
| --accent | #956400 | primary action |
| --success | #346538 | ready state |
| --danger | #9F2F2D | error state |

## 3. Typography
- Sans: `Helvetica Neue`, `Arial`, sans-serif.
- Editorial heading: Georgia, serif.
- Body minimum: 14px; caption: 12px; heading uses clamp for narrow windows.

## 4. Spacing & Layout
- Base unit: 4px.
- Desktop shell: max-width 1180px, two-column workspace when an image is loaded.
- Mobile breakpoint: 760px; panels stack and controls remain full width.

## 5. Primitives
- `DropZone`: empty, drag-over, loaded, error states; dashed line and tonal wash, no accent state border.
- `Panel`: title, content, muted metadata; 1px neutral line.
- `StatusPill`: checking, ready, unavailable; text and tonal wash only.
- `PrimaryButton`: idle, disabled, busy; dark ink background with focus-visible outline.
- `DescriptionEditor`: clean, dirty, error; preserves edited text.
- `SettingsForm`: subscription access status, API fallback fields, secure credential actions, and persisted preset rows.

## 6. Interaction & Motion
- Only transform/opacity transitions; no decorative motion.
- Drag-over uses a tonal background wash and status copy.
- Respect `prefers-reduced-motion` by removing transitions.

## 7. Accessibility
- Native buttons and labels, visible focus-visible outlines, status announcements, keyboard-accessible drop fallback.
- No color-only state communication; every state has text.
- Subscription and API access are separate states; unavailable OAuth is explicit and never represented as a working sign-in action.

## 8. Accepted Debt
- Native Tauri dialog and drag-drop events are wired in the workspace; browser fallback remains an explicit file input.
- OAuth plan usage for ChatGPT Plus and SuperGrok remains provider-gated until official provider flow adapters are implemented.
