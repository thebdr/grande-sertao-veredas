// Typing as a keyboard types, for the tests (the spike's points 4 and 7): Spix posts a key event
// with a key code and no text, or text and no key code, and Chromium (Monaco in the embedded page)
// takes neither as typing. This posts, for each character, a press and a release carrying both,
// to the item's window, so the keys travel the keyboard's path through Qt into the page.
#pragma once

#include <QGuiApplication>
#include <QKeyEvent>
#include <QObject>
#include <QQuickItem>
#include <QQuickWindow>

class OficinaKeys : public QObject
{
    Q_OBJECT
public:
    using QObject::QObject;

    Q_INVOKABLE void type(QQuickItem *target, const QString &text)
    {
        if (!target || !target->window())
            return;
        target->forceActiveFocus();
        for (const QChar c : text) {
            const ushort u = c.unicode();
            // Qt's key codes for ASCII are the upper-case character's code; others have none.
            int key = Qt::Key_unknown;
            if (u == '\n')
                key = Qt::Key_Return;
            else if (u >= 'a' && u <= 'z')
                key = u - 'a' + 'A';
            else if (u >= 0x20 && u < 0x7f)
                key = u;
            const QString t = u == '\n' ? QStringLiteral("\r") : QString(c);
            QGuiApplication::postEvent(target->window(), new QKeyEvent(QEvent::KeyPress, key, Qt::NoModifier, t));
            QGuiApplication::postEvent(target->window(), new QKeyEvent(QEvent::KeyRelease, key, Qt::NoModifier, t));
        }
    }
};
