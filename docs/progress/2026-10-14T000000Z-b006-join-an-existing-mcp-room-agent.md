- 2026-10-14 — Join an existing MCP room: Agent access gained Copy room link and Use room
  token (bare token, MCP URL or room link; invalid input toasts and changes nothing), and
  a `#room=` fragment joins on startup and `hashchange`, consumed with
  `history.replaceState` so the token leaves the address bar. Pure helper tests with five
  mutation controls and a Playwright second-browser join test shipped; full
  `scripts/check.sh --web` gate and 3 e2e tests passed. Menu Apply itself not e2e-driven;
  Safari/Firefox and deployed hibernation unverified.
  (Superseded 2026-10-15: the fragment path now asks before joining — see the
  room-link join guard entry.)
