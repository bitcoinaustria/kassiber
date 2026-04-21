import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

import "components"
import "dialogs"
import "pages"

ApplicationWindow {
    id: root
    objectName: "mainWindow"
    readonly property bool previewSceneMode: (uiPreviewPage || "") !== ""
    readonly property bool captureMode: !!uiCaptureMode
    readonly property bool standaloneWelcomeMode: dashboardVM.currentPage === "welcome" && (!dashboardVM.hasProfile || previewSceneMode)
    property bool hideSensitivePreview: false
    property string previewLanguage: "EN"

    visible: false
    title: dashboardVM.windowTitle
    width: windowState.width > 0 ? windowState.width : 1240
    height: windowState.height > 0 ? windowState.height : 820
    minimumWidth: 1060
    minimumHeight: 700
    color: standaloneWelcomeMode ? theme.warmBg : theme.bg

    AddConnectionDialog {
        id: addConnectionDialog
    }

    SettingsDialog {
        id: settingsDialog
    }

    Connections {
        target: dashboardVM
        function onPageChanged() {
            if (addConnectionDialog.opened) {
                addConnectionDialog.close()
            }
        }
    }

    Popup {
        id: projectPopup
        width: 360
        x: root.width - width - 18
        y: 52
        padding: 0
        modal: false
        focus: true
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
        background: Rectangle {
            color: theme.paper
            border.color: theme.ink
            border.width: 1
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            Rectangle {
                Layout.fillWidth: true
                height: 40
                color: "transparent"
                border.color: theme.line
                border.width: 0

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 1
                    color: theme.line
                }

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 14
                    anchors.rightMargin: 14

                    Text {
                        text: "Switch profile"
                        color: theme.ink3
                        font.family: theme.monoFont
                        font.pixelSize: 9
                        font.bold: true
                        font.letterSpacing: 1.4
                    }

                    Item {
                        Layout.fillWidth: true
                    }

                    Text {
                        text: "MANAGE \u2192"
                        color: theme.accent
                        font.family: theme.monoFont
                        font.pixelSize: 9
                        font.bold: true
                        font.letterSpacing: 1.4
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0

                Repeater {
                    model: dashboardVM.availableProfiles

                    Button {
                        Layout.fillWidth: true
                        implicitHeight: 56
                        flat: true

                        contentItem: RowLayout {
                            spacing: 10

                            Rectangle {
                                width: 8
                                height: 8
                                radius: 4
                                color: modelData["current"] === "yes" ? theme.accent : "transparent"
                                border.color: modelData["current"] === "yes" ? theme.accent : theme.line
                                border.width: 1
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2

                                Text {
                                    Layout.fillWidth: true
                                    text: modelData["label"] || ""
                                    color: theme.ink
                                    font.family: theme.serifFont
                                    font.pixelSize: 15
                                    elide: Text.ElideRight
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: (modelData["tax_country"] || "").toUpperCase() + "  \u00b7  " + (modelData["gains_algorithm"] || "")
                                    color: theme.ink3
                                    font.family: theme.monoFont
                                    font.pixelSize: 10
                                    elide: Text.ElideRight
                                }
                            }
                        }

                        background: Rectangle {
                            color: modelData["current"] === "yes" ? theme.paper2 : "transparent"
                            border.color: theme.line
                            border.width: 0

                            Rectangle {
                                visible: index > 0
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.top: parent.top
                                height: 1
                                color: theme.line
                            }
                        }
                    }
                }
            }
        }
    }

    Rectangle {
        visible: previewSceneMode && !captureMode
        z: 20
        x: 12
        y: 12
        color: theme.paper2
        border.color: theme.ink
        border.width: 1

        RowLayout {
            anchors.fill: parent
            anchors.margins: 6
            spacing: 4

            Text {
                text: "SCENE"
                color: theme.ink3
                font.family: theme.monoFont
                font.pixelSize: 9
                font.letterSpacing: 1.0
            }

            Repeater {
                model: [
                    { "id": "welcome", "label": "Welcome" },
                    { "id": "overview-empty", "label": "Overview \u00b7 empty" },
                    { "id": "overview-data", "label": "Overview \u00b7 data" },
                    { "id": "transactions", "label": "Transactions" },
                    { "id": "tax", "label": "Tax \u00b7 capital gains" },
                    { "id": "connection-detail", "label": "Connection detail" },
                    { "id": "profiles", "label": "Profiles" }
                ]

                Button {
                    flat: true
                    leftPadding: 8
                    rightPadding: 8
                    topPadding: 3
                    bottomPadding: 3
                    onClicked: dashboardVM.selectPage(modelData["id"])

                    contentItem: Text {
                        text: modelData["label"]
                        color: {
                            var current = dashboardVM.currentPage
                            var wanted = modelData["id"]
                            var active = current === wanted
                            if ((wanted === "overview-empty" || wanted === "overview-data") && current === "overview") {
                                active = true
                            }
                            if (wanted === "tax" && current === "reports") {
                                active = true
                            }
                            return active ? theme.paper : theme.ink
                        }
                        font.family: theme.monoFont
                        font.pixelSize: 10
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }

                    background: Rectangle {
                        color: {
                            var current = dashboardVM.currentPage
                            var wanted = modelData["id"]
                            var active = current === wanted
                            if ((wanted === "overview-empty" || wanted === "overview-data") && current === "overview") {
                                active = true
                            }
                            if (wanted === "tax" && current === "reports") {
                                active = true
                            }
                            return active ? theme.ink : "transparent"
                        }
                    }
                }
            }
        }
    }

    WelcomePage {
        anchors.fill: parent
        visible: root.standaloneWelcomeMode
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        visible: !root.standaloneWelcomeMode

        AppHeader {
            Layout.fillWidth: true
            currentRoute: dashboardVM.currentPage
            workspaceLabel: dashboardVM.currentWorkspaceLabel
            profileLabel: dashboardVM.currentProfileLabel
            currentLang: root.previewLanguage
            hideSensitive: root.hideSensitivePreview
            showLang: true
            onRouteSelected: dashboardVM.selectPage(id)
            onWorkspaceClicked: projectPopup.open()
            onLangSelected: root.previewLanguage = code
            onHideSensitiveToggled: root.hideSensitivePreview = !root.hideSensitivePreview
            onSettingsClicked: settingsDialog.open()
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            color: theme.paper

            StackLayout {
                anchors.fill: parent
                anchors.margins: 0
                currentIndex: dashboardVM.pageIndex

                WelcomePage {
                }

                OverviewPage {
                    hideSensitive: root.hideSensitivePreview
                    onRequestAddConnection: addConnectionDialog.open()
                }

                ConnectionDetailPage {
                    onRequestAddConnection: addConnectionDialog.open()
                }

                TransactionsPage {
                }

                ReportsPage {
                    hideSensitive: root.hideSensitivePreview
                    onRequestBack: dashboardVM.selectPage("overview")
                }

                SettingsPage {
                }

                ProfilesPage {
                    onRequestBack: dashboardVM.selectPage("overview")
                }
            }
        }

        AppFooter {
            Layout.fillWidth: true
            versionText: settingsVM.versionText
        }
    }
}
