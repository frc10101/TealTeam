// Persistent device identity (A5).
//
// Every browser gets a permanent UUID, independent of who is signed in on it, so
// a lead scout can assign a robot to "the tablet in the stands" rather than to a
// person who might hand it over between matches.
//
// The id lives in localStorage (durable) and is mirrored into a long-lived
// cookie, because the SERVER has to be able to read it on ordinary page requests
// and localStorage is not visible server-side.
(function () {
  "use strict";

  var STORAGE_KEY = "tt_device_uuid";
  var COOKIE = "tt_device";
  var TEN_YEARS = 315360000;
  var HEARTBEAT_MS = 60000;

  function newId() {
    if (window.crypto && crypto.randomUUID) return crypto.randomUUID();
    // Older browsers on borrowed tablets: unique enough for this purpose.
    return "dev-" + Date.now().toString(36) + "-" + Math.random().toString(36).slice(2, 12);
  }

  var deviceId;
  try {
    deviceId = localStorage.getItem(STORAGE_KEY);
    if (!deviceId) {
      deviceId = newId();
      localStorage.setItem(STORAGE_KEY, deviceId);
    }
  } catch (e) {
    // Private browsing, or storage disabled. A per-session id still lets the
    // lead scout see the tablet while it is being used.
    deviceId = newId();
  }

  document.cookie =
    COOKIE + "=" + encodeURIComponent(deviceId) +
    ";path=/;max-age=" + TEN_YEARS + ";samesite=lax";

  function heartbeat() {
    // Failure is expected and uninteresting: the network drops constantly at an
    // event. Swallow it rather than filling a scout's console with red.
    fetch("/api/device/heartbeat", { method: "POST", credentials: "same-origin" })
      .catch(function () {});
  }

  heartbeat();
  setInterval(heartbeat, HEARTBEAT_MS);
})();
