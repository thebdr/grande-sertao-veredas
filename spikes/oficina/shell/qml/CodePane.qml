// The ORCL editor (the spike's point 4): Monaco in a page Oficina ships, joined by a web channel to
// the language-server client. `evaluate` is Oficina's hook into its own page: a test reads the page
// through it, never through a script the page carries for tests.
import QtQuick
import QtQuick.Controls
import QtWebEngine
import QtWebChannel
import Oficina

Rectangle {
    id: pane
    objectName: "codePane"
    color: "#0F0F11"

    property string probe: ""

    function open(path) {
        client.open(path);
    }

    // Type into the page as a keyboard does (the tests' hook, cpp/keys.h).
    function typeText(text) {
        oficinaKeys.type(code, text);
    }

    function evaluate(script) {
        code.runJavaScript(script, function (r) {
            pane.probe = (r === undefined || r === null) ? "" : (typeof r === "string" ? r : JSON.stringify(r));
        });
    }

    LanguageClient {
        id: client
        objectName: "languageClient"
        WebChannel.id: "oficina"
    }

    WebChannel {
        id: channel
        registeredObjects: [client]
    }

    Text {
        id: status
        objectName: "codeStatus"
        anchors { left: parent.left; top: parent.top; margins: 4 }
        text: client.status
        textFormat: Text.PlainText
        color: "#9A9AA3"
    }

    WebEngineView {
        id: code
        objectName: "codeView"
        anchors { left: parent.left; right: parent.right; top: status.bottom; bottom: parent.bottom }
        url: client.pageUrl
        webChannel: channel
    }
}
