# Chat align v2 (2026-08-10)

Populated session + pixel QA after nesting Send in the compose chrome.

## Findings from populated transcript
- Side-by-side Send looked taller because the field's 1px border shrank the white fill (~38) vs solid Send fill (40) at the same outer height.
- User vs assistant prose started on different left edges (`border_l_2+pl_4` vs `pl_1`).
- Hover-copy icons could shift row width when revealed.

## Fixes shipped
- Send/attach live inside one 40px bordered chrome (shared top/bottom border pixels).
- Shared accent-rail + content pad for user/assistant/thinking/tools; fixed copy slot width.
- Slightly tighter turn padding.

## Screenshots
- `chat-populated-align-v2.png` — full window
- `chat-transcript-align-v2.png` — transcript column
- `chat-composer-align-v2.png` — composer band
- `chat-composer-send-tight-v2.png` — Send/field junction (pixel-flush)
