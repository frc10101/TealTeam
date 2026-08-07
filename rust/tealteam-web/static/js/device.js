// Persistent device identity: every browser gets a permanent UUID stored in
// localStorage, mirrored into a cookie, and heartbeats to the server so lead
// scouts can assign robots to specific devices.
(function () {
    var KEY = 'tealteam_device_uuid';
    var deviceId = null;

    try {
        deviceId = localStorage.getItem(KEY);
        if (!deviceId) {
            deviceId = (crypto.randomUUID && crypto.randomUUID()) ||
                'dev-' + Date.now().toString(36) + '-' + Math.random().toString(36).slice(2, 10);
            localStorage.setItem(KEY, deviceId);
        }
    } catch (e) {
        // localStorage unavailable (private mode); fall back to a per-session id.
        deviceId = 'tmp-' + Math.random().toString(36).slice(2, 12);
    }

    // Ten-year cookie so the server can read the device id on normal requests.
    document.cookie = 'device_uuid=' + encodeURIComponent(deviceId) +
        ';path=/;max-age=315360000;samesite=lax';

    function heartbeat() {
        fetch('/api/device/heartbeat', { method: 'POST' }).catch(function () {
            /* offline — ignore */
        });
    }

    heartbeat();
    setInterval(heartbeat, 60000);
})();
