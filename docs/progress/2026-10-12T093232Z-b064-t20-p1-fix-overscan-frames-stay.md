- 2026-10-12 — T20 P1 fix: overscan frames stay within the pixel cap (32 Mpx native, 16 Mpx web)
  and the texture limit by rendering an over-cap canvas at lower resolution; one planner decides
  requests, so an over-limit canvas no longer re-requests every update; web-build.sh exits 0
  without the CJK font. Tests seen failing under mutations. Not verified: over-limit canvas by eye.
