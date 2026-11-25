// Oficina's window: the workbench, its panes docked by KDDockWidgets. The project pane first; the
// editors, the inspector and the rest arrive with their clauses (docs/oficina-plan.md).
import QtQuick
import QtQuick.Controls
import com.kdab.dockwidgets 2.0 as KDDW

ApplicationWindow {
    id: window
    objectName: "oficina"
    visible: true
    width: 1280
    height: 800
    title: "Oficina"
    color: "#141416"

    KDDW.DockingArea {
        id: area
        objectName: "area"
        anchors.fill: parent
        uniqueName: "workbench"

        // The docks as they stand, for a test to read (the tests' driver reaches items, not the
        // docks themselves): each dock's name, open, floating, whether its pane sits inside this
        // docking area (its parents reach it) and is drawn in this window, set by `describeDocks`.
        property string docks: ""

        function inside(item, ancestor) {
            for (let p = item; p; p = p.parent) {
                if (p === ancestor)
                    return true;
            }
            return false;
        }

        // What a person does with a dock's title bar and the window's menu: close it, open it.
        function closeDock(name) {
            dockNamed(name).close();
        }

        function openDock(name) {
            dockNamed(name).open();
        }

        function dockNamed(name) {
            return [projectDock].find((d) => d.uniqueName === name);
        }

        function describeDocks() {
            docks = JSON.stringify([[projectDock, projectPane]].map(([dock, pane]) => [dock.uniqueName, dock.isOpen, dock.isFloating, inside(pane, area), pane.Window.window === window]));
        }

        KDDW.DockWidget {
            id: projectDock
            uniqueName: "project"
            title: qsTr("Project")
            ProjectPane {
                id: projectPane
                objectName: "projectPane"
                anchors.fill: parent
            }
        }

        Component.onCompleted: addDockWidget(projectDock, KDDW.KDDockWidgets.Location_OnLeft, null, Qt.size(320, 800))
    }
}
