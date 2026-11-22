// An embedded page (the spike's points 4, 5 and 7): what the page holds is read through a hook on
// Oficina's side, never through a script the page carries.
import QtQuick
import QtQuick.Controls
import QtWebEngine

Rectangle {
    id: pane
    objectName: "webPane"
    color: "#1B1B1E"

    property string pageText: ""
    property string url: "data:text/html,<title>probe</title><p id=x>hello from the page</p>"

    // Read an element's text by its id into `pageText`.
    function readText(id) {
        web.runJavaScript("(document.getElementById(" + JSON.stringify(id) + ") || {}).textContent || ''", function (r) {
            pane.pageText = r;
        });
    }

    WebEngineView {
        id: web
        objectName: "web"
        anchors.fill: parent
        url: pane.url
    }
}
