"""One persona-only local scheduler. Native alone decides whether time is due."""
from __future__ import annotations

import asyncio
import heapq
import logging
import time

from .tzdb_loader import canonical_timezone, freeze_persona_time, load_manifest

logger = logging.getLogger(__name__)
_CEILINGS = {"active": 300_000, "asleep": 3_600_000, "calm": 21_600_000}


class EmbodimentClock:
    def __init__(self, *, bridge, persona_locks, config, now_ms=None):
        self._bridge = bridge
        self._locks = persona_locks
        self._config = config
        self._now = now_ms or (lambda: int(time.time() * 1000))
        self._heap = []
        self._deadlines = {}
        self._scopes = {}
        self._wake = asyncio.Event()
        self._task = None

    @staticmethod
    def key(scope):
        return scope["bot_token"], scope["persona_token"]

    async def start(self):
        if self._task is None:
            self._task = asyncio.create_task(self._run(), name="AstrEmbodimentClock")

    async def stop(self):
        task, self._task = self._task, None
        if task is not None:
            task.cancel()
            try:
                await task
            except asyncio.CancelledError:
                pass
        self._heap.clear()
        self._deadlines.clear()
        self._scopes.clear()

    def _schedule(self, scope, deadline):
        # All heap operations are synchronous on the one Host event loop and
        # occur outside persona locks. Keep exactly one heap entry per scope.
        key = self.key(scope)
        self._scopes[key] = dict(scope)
        self._deadlines[key] = deadline
        self._heap = [(v, k) for k, v in self._deadlines.items()]
        heapq.heapify(self._heap)
        self._wake.set()

    async def notify_persona(self, scope):
        # The running task performs the same authenticated lookup/create path.
        self._schedule(scope, self._now())

    async def rescan(self):
        for _ in range(3):
            token, after, scopes = None, None, {}
            try:
                while True:
                    page = self._bridge.list_embodiment_personas_v1({
                        "schema_version": 1, "limit": 64,
                        "snapshot_token": token, "after": after,
                    })
                    token = page["snapshot_token"]
                    for entry in page["entries"]:
                        scope = entry["scope"]
                        scopes[self.key(scope)] = scope
                    after = page["next_after"]
                    if after is None:
                        break
                    await asyncio.sleep(0)
            except Exception as exc:
                if "INVENTORY_CHANGED" in str(exc):
                    continue
                raise
            now = self._now()
            for key in set(self._scopes) - set(scopes):
                self._scopes.pop(key, None)
                self._deadlines.pop(key, None)
            for key, scope in scopes.items():
                self._schedule(scope, self._deadlines.get(key, now))
            return
        raise RuntimeError("INVENTORY_CHANGED")

    @staticmethod
    def _minute(value):
        hour, minute = map(int, str(value).split(":"))
        if not 0 <= hour < 24 or not 0 <= minute < 60:
            raise ValueError("INVALID_PERSONA_SLEEP_TIME")
        return hour * 60 + minute

    def _desired(self, current=None):
        manifest = load_manifest()
        profile = dict(current["profile"]) if current else {
            "schema_version": 1, "persona_tzid": "America/Los_Angeles",
            "tzdb_release": manifest["tzdb_release"],
            "tzdb_content_sha256": manifest["content_sha256"],
            "circadian_period_millis": 86_400_000,
            "homeostatic_awake_gain_per_hour": 50_000,
            "homeostatic_asleep_decay_per_hour": 100_000,
            "drowsy_enter_threshold": 700_000, "drowsy_exit_threshold": 300_000,
            "endogenous_phase_hysteresis": 50_000,
            "maximum_analytic_horizon_ms": 604_800_000,
        }
        schedule = dict(current["schedule"]) if current else {
            "schema_version": 1, "mode": "auto", "chronotype": "night_owl",
            "preferred_sleep_local_minute": 90, "preferred_wake_local_minute": 570,
            "sleep_flex_minutes": 120, "entrainment_rate_minutes_per_day": 90,
        }
        profile["persona_tzid"] = canonical_timezone(str(self._config("persona_timezone", profile["persona_tzid"])))
        sleep = self._config("persona_sleep", {})
        mode = sleep.get("mode", "auto")
        if mode not in {"auto", "fixed"}:
            raise ValueError("INVALID_PERSONA_SLEEP_MODE")
        schedule["mode"] = mode
        if mode == "fixed":
            schedule["preferred_sleep_local_minute"] = self._minute(sleep.get("sleep", "01:30"))
            schedule["preferred_wake_local_minute"] = self._minute(sleep.get("wake", "09:30"))
        return profile, schedule

    def prepare_locked(self, scope):
        """Caller owns the shared persona lock; this method contains no awaits."""
        lookup = self._bridge.get_embodiment_persona_v1(scope)
        if lookup["status"] == "missing":
            profile, schedule = self._desired()
            request = self._bridge.compile_core_host_request_v1("create", {
                "schema_version": 1, "operation_id": "00" * 16, "scope": scope,
                "incarnation_digest": lookup["incarnation_digest"],
                "profile_template": profile, "sleep_schedule": schedule,
                "frozen_creation": freeze_persona_time(profile["persona_tzid"], self._now()),
            })
            self._bridge.create_embodiment_persona_if_missing_v1(request)
        elif lookup["status"] != "present":
            raise ValueError("INVALID_PERSONA_LOOKUP")
        current = self._bridge.read_embodiment_profile_v1(scope)
        profile, schedule = self._desired(current)
        if profile != current["profile"] or schedule != current["schedule"]:
            request = self._bridge.compile_core_host_request_v1("profile_cas", {
                "schema_version": 1, "operation_id": "00" * 16, "scope": scope,
                "expected_profile_revision": current["profile_revision"],
                "expected_schedule_revision": current["schedule_revision"],
                "replacement_profile": profile, "replacement_schedule": schedule,
                "frozen": freeze_persona_time(profile["persona_tzid"], self._now()),
            })
            self._bridge.compare_and_swap_embodiment_profile_v1(request)
            current = self._bridge.read_embodiment_profile_v1(scope)
        return current

    async def tick(self, scope):
        key = self.key(scope)
        async with self._locks.setdefault(key, asyncio.Lock()):
            current = self.prepare_locked(scope)
            frozen = freeze_persona_time(current["profile"]["persona_tzid"], self._now())
            status = self._bridge.embodiment_clock_status_v1({
                "schema_version": 1, "scope": scope,
                "profile_revision": current["profile_revision"],
                "schedule_revision": current["schedule_revision"], "frozen": frozen,
            })
            if status["status"] == "due":
                result = self._bridge.advance_embodiment_time_v1(status["exact_advance_request_bytes"])
                state = result["receipt"]["result"]
            elif status["status"] == "not_due":
                state = status
            else:
                raise ValueError("INVALID_CLOCK_STATUS")
            deadline = frozen["now_utc_ms"] + max(60_000, min(
                int(state["next_due_at_utc_ms"]) - frozen["now_utc_ms"],
                _CEILINGS[state["cadence_class"]],
            ))
        self._schedule(scope, deadline)

    async def _run(self):
        rescan_at = 0
        while True:
            now = self._now()
            if now >= rescan_at:
                try:
                    await self.rescan()
                    rescan_at = now + 300_000
                except Exception as exc:
                    logger.warning("Embodiment inventory unavailable: %s", type(exc).__name__)
                    rescan_at = now + 60_000
            if self._heap and self._heap[0][0] <= now:
                _, key = heapq.heappop(self._heap)
                self._deadlines.pop(key, None)
                scope = self._scopes[key]
                try:
                    await self.tick(scope)
                except Exception as exc:
                    logger.warning("Embodiment clock unavailable: %s", type(exc).__name__)
                    self._schedule(scope, self._now() + 3_600_000)
                await asyncio.sleep(0)
                continue
            deadline = min(rescan_at, self._heap[0][0] if self._heap else rescan_at)
            self._wake.clear()
            try:
                await asyncio.wait_for(self._wake.wait(), max(0.001, (deadline - self._now()) / 1000))
            except asyncio.TimeoutError:
                pass
