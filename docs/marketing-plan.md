# SweepMac — Marketing & Distribution Plan

Researched Nov 2026. Channels ranked by expected ROI for a free, open-source Mac cleaner.

## Tier 1 — Do this week (highest ROI, no karma/account gates)

### 1. Hacker News — Show HN
- **Why:** No karma gate (new accounts can post). Massive dev traffic for Rust/egui tools. Perfect audience.
- **How:** Paste title + body from `docs/show-hn.md` at https://news.ycombinator.com/showhn.html
- **Rules:** Link must be something people can try (GitHub repo works). No landing pages/fundraisers. Explain tradeoffs.
- **Timing:** US weekday morning (9-11 AM ET). Reply to every comment for the first 2 hours.

### 2. Dev.to article
- **Why:** Long-tail Google traffic for months. "How I built a Mac cleaner in Rust" ranks for "rust mac app", "mac cleaner open source".
- **How:** Publish `docs/devto-article.md` (already drafted, frontmatter set). Add 2-3 screenshots.
- **Timing:** Same week as HN launch for cross-pollination.

### 3. MacNative directory
- **Why:** 229 curated Mac apps, visitors are actively browsing for Mac software. Free listing (reviewed).
- **How:** Submit at https://macnative.io/submit — clear description, screenshots, pricing (free), macOS 13+, Apple Silicon.
- **Note:** Pro tier adds featured placement + newsletter promotion. Free listing is fine to start.

### 4. Awesome Mac list (jaywcjlove/awesome-mac — 117K ⭐)
- **Why:** The canonical macOS software reference list. Being listed = permanent discoverability.
- **How:** Fork → add SweepMac under "Developer Tools" or "Utilities" category → PR following their CONTRIBUTING.md format.
- **Format:** factual one-line description + official website. Check if already listed first.
- **Repo:** https://github.com/jaywcjlove/awesome-mac

### 5. AlternativeTo
- **Why:** People searching "CleanMyMac alternative" land here. SweepMac already solves the exact problem.
- **How:** Search for CleanMyMac entry → submit SweepMac as alternative. Explain differences (free, open-source, trash-first, no subscription).
- **URL:** https://alternativeto.net/faq/

## Tier 2 — Do this month (good ROI, some effort)

### 6. Product Hunt
- **Why:** Launch-day visibility, community-driven, good for broadly appealing tools.
- **How:** https://producthunt.com/launch — strong visual demo (20-sec video), tagline, reply to every comment. No upvote asking.
- **Timing:** Pick a day you can be online all day. Tuesday-Thursday best.

### 7. SemiLaunch
- **Why:** Mac-specific launch platform, free first launch, 7-day visibility window.
- **How:** https://semilaunch.com/launch/mac-apps — submit name + website, reviewed before going live.
- **Note:** Smaller audience than PH but Mac-targeted.

### 8. Mac newsletters (pitch, don't expect)
- **Native Mac Apps** (free) — https://nativemacapps.substack.com/ — native quality apps
- **MacStories Weekly** (paid club, $5/mo) — https://www.macstories.net/club/ — Mac apps + workflows
- **Six Colors** (mostly free) — https://sixcolors.com/ — Apple news + app coverage
- **TidBITS** (free) — https://tidbits.com/mailing-lists/ — Mac app updates via Watchlist
- **The Sweet Setup** (free) — https://thesweetsetup.com/ — "best apps" roundups
- **Daring Fireball** (free blog) — https://daringfireball.net/ — high-profile app links
- **Pitch format:** "You recently covered [X]. My Mac app does [Y] — here's a 20-sec demo. Happy to provide review access."

### 9. YouTube demo video
- **Why:** 2-min demo = huge for Mac tools. Shows up in Google search for "mac cleaner" queries.
- **How:** Screen recording: scan → show results → clean → undo from Trash. Keep it under 2 min. Upload with title "SweepMac — free open-source Mac cleaner (Rust)".

## Tier 3 — Ongoing (compound over time)

### 10. GitHub discoverability
- Topics set ✓ (rust, macos, egui, open-source, cleaner, clamav, desktop-app)
- Add screenshots to README (drag images into repo)
- Add a "star" badge + download count badge
- Consider a GitHub Pages landing page

### 11. Reddit (karma-gated)
- Build 20-50 comment karma first (1-2 weeks of genuine comments)
- Then retry SweepMac comments in r/macapps, r/macOS, r/SideProject
- Launch threads: r/SideProject (promo-friendly), r/macapps (mod review), r/rust (dev story)

### 12. Twitter/X consistency
- Post weekly from @ankush_thxkur
- "Share on X" button in v0.2.3 lets users amplify
- Engage with #rustlang #macos #opensource conversations

### 13. Open-source directories
- **awesome-rust** — if SweepMac qualifies as a "GUI" or "system" example
- **open-saas-directory/awesome-native-macosx-apps** — native apps directory (self-submissions restricted to exceptional apps)
- **Homebrew cask** — `brew install --cask sweepmac` would be huge for Mac users. Requires a signed/notarized build or a formula that builds from source.

## What NOT to do
- Don't buy upvotes, fake reviews, or use multiple accounts — Reddit/X shadowban for this
- Don't spam the same post across 20 subreddits in one day
- Don't post identical content on every platform — adapt the angle per audience
- Don't ask for upvotes on Product Hunt (explicitly prohibited)

## Priority order (if you only have 2 hours)
1. Show HN post (30 min) → highest traffic
2. Dev.to article (30 min) → long-tail SEO
3. MacNative submission (15 min) → Mac-specific audience
4. Awesome Mac PR (15 min) → permanent reference listing
5. AlternativeTo submission (10 min) → replacement seekers
6. GitHub README screenshots (10 min) → conversion rate

## What I can do for you right now
- Draft the Show HN post ✓ (docs/show-hn.md)
- Draft the Dev.to article ✓ (docs/devto-article.md)
- Draft Product Hunt tagline + description
- Draft the awesome-mac PR entry
- Draft the AlternativeTo submission
- Write the Dev.to article as a full publishable piece
- Create a 2-min YouTube script/storyboard

Tell me which ones to draft next.