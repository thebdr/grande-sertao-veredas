// An embedded page (the spike's points 5 and 7): a page Oficina does not ship, a supervisão page
// say, shown with no web channel, so it finds no bridge object (the review's entry 32). What the
// page holds is read through a hook on Oficina's side, never through a script the page carries.
import QtQuick
import QtQuick.Controls
import QtWebEngine

Rectangle {
    id: pane
    objectName: "webPane"
    color: "#1B1B1E"

    property string pageText: ""
    property string probe: ""
    property string url: "data:text/html,<title>probe</title><p id=x>hello from the page</p>"

    function load(address) {
        pane.url = address;
    }

    // Read an element's text by its id into `pageText`.
    function readText(id) {
        web.runJavaScript("(document.getElementById(" + JSON.stringify(id) + ") || {}).textContent || ''", function (r) {
            pane.pageText = r;
        });
    }

    function answer(r) {
        pane.probe = (r === undefined || r === null) ? "" : (typeof r === "string" ? r : JSON.stringify(r));
    }

    // Oficina's hook, in a world of its own: the page's document is read, while the page's scripts
    // can neither see the hook nor stand in for what it reads with globals of theirs.
    function evaluate(script) {
        web.runJavaScript(script, WebEngineScript.ApplicationWorld, pane.answer);
    }

    // The same hook in the page's own world, where its scripts run and where a bridge object would
    // be injected: what the page itself would find.
    function evaluateInPage(script) {
        web.runJavaScript(script, WebEngineScript.MainWorld, pane.answer);
    }

    WebEngineView {
        id: web
        objectName: "web"
        anchors.fill: parent
        url: pane.url
    }
}
