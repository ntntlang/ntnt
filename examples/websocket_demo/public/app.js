// Browser side of examples/websocket_demo/server.tnt.
const url = (path) => (location.protocol === "https:" ? "wss://" : "ws://") + location.host + path;
const log = (id, line) => {
  const el = document.getElementById(id);
  if (el) el.textContent += line + "\n";
};

// One owner per socket path. It reconnects when the server closes: lifetime
// limit (1001), development reload (1012) and overload (1013) are all normal.
// 1008 means the client sent too much; don't hammer the server. send() always
// uses the current socket, so there is never more than one socket or retry
// timer per path.
function channel(path, logId) {
  let ws = null;
  let retry = null;
  const open = () => {
    retry = null;
    const socket = new WebSocket(url(path));
    ws = socket;
    socket.onmessage = (e) => log(logId, e.data);
    socket.onclose = (e) => {
      if (socket !== ws) return;
      log(logId, "closed " + e.code + (e.reason ? " " + e.reason : ""));
      if (e.code !== 1008) retry = setTimeout(open, 1000);
    };
  };
  open();
  return {
    send(data) {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(data);
        return;
      }
      log(logId, "not sent: not connected");
      // After 1008 nothing reconnects by itself; a deliberate send does.
      if (ws.readyState === WebSocket.CLOSED && retry === null) open();
    },
  };
}

const echo = channel("/ws/echo", "echo-log");
document.getElementById("echo-send").onclick = () => {
  echo.send(document.getElementById("echo-text").value);
};

if (document.getElementById("feed-log")) {
  channel("/ws/feed", "feed-log");
  document.getElementById("announce").onsubmit = async (e) => {
    e.preventDefault();
    const form = e.target;
    const draft = form.text.value;
    try {
      const response = await fetch("/announce", { method: "POST", body: new URLSearchParams(new FormData(form)) });
      if (!response.ok) {
        log("feed-log", "not published: " + response.status + " " + (await response.text()));
        return;
      }
    } catch (error) {
      log("feed-log", "not published: " + error.message);
      return;
    }
    // Clear only what was published, not anything typed since.
    if (form.text.value === draft) form.text.value = "";
  };
}
