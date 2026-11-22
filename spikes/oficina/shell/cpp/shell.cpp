#include "oficina-spike-shell/cpp/shell.h"
#include "oficina-spike-shell/cpp/keys.h"

#include <memory>
#include <string>
#include <vector>

#include <QGuiApplication>
#include <QOffscreenSurface>
#include <QOpenGLContext>
#include <QOpenGLFunctions>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QVariantMap>
#include <QtWebEngineQuick/QtWebEngineQuick>

#include <kddockwidgets/Config.h>
#include <kddockwidgets/qtquick/Platform.h>

#include <Spix/AnyRpcServer.h>
#include <Spix/QtQmlBot.h>

namespace {

// What will draw the windows, asked before the first one (the spike's point 6): the renderer of an
// OpenGL context made as Qt Quick makes its own on this platform, or none where the platform offers
// no OpenGL, Qt Quick then drawing in software itself. On Windows Qt Quick draws through Direct3D
// 11, whose adapter (WARP on a machine with no GPU) this probe does not ask: a remainder.
struct Drawing {
    QString adapter;
    bool software;
};

// Renderers that are a processor drawing, not a GPU: Mesa's, Chromium's, Windows' own.
constexpr const char *SOFTWARE[] = {"llvmpipe", "softpipe", "swrast", "SwiftShader", "GDI Generic", "Microsoft Basic Render", "WARP"};

Drawing probeDrawing()
{
    QOpenGLContext context;
    QOffscreenSurface surface;
    if (!context.create())
        return {QStringLiteral("no OpenGL (Qt Quick's software renderer)"), true};
    surface.setFormat(context.format());
    surface.create();
    if (!context.makeCurrent(&surface))
        return {QStringLiteral("no OpenGL (Qt Quick's software renderer)"), true};
    const auto *renderer = reinterpret_cast<const char *>(context.functions()->glGetString(GL_RENDERER));
    const QString adapter = QString::fromLatin1(renderer ? renderer : "");
    context.doneCurrent();
    bool software = adapter.isEmpty();
    for (const char *name : SOFTWARE)
        software = software || adapter.contains(QLatin1String(name), Qt::CaseInsensitive);
    return {adapter, software};
}

} // namespace

::std::int32_t oficina_run(::rust::Vec<::rust::String> args)
{
    // WebEngine shares its OpenGL context with Qt Quick: it is set up before the application.
    QtWebEngineQuick::initialize();

    std::vector<std::string> owned;
    for (const auto &a : args)
        owned.emplace_back(a.data(), a.size());
    std::vector<char *> argv;
    for (auto &a : owned)
        argv.push_back(a.data());
    argv.push_back(nullptr);
    int argc = static_cast<int>(owned.size());

    QGuiApplication app(argc, argv.data());
    QGuiApplication::setApplicationName(QStringLiteral("Oficina"));

    // Drawing in software: Chromium told to leave the GPU alone, before WebEngine's first page reads
    // its flags; the QML switches its effects off and says so.
    const Drawing drawing = probeDrawing();
    if (drawing.software) {
        QByteArray flags = qgetenv("QTWEBENGINE_CHROMIUM_FLAGS");
        flags.append(" --disable-gpu");
        qputenv("QTWEBENGINE_CHROMIUM_FLAGS", flags.trimmed());
    }

    KDDockWidgets::initFrontend(KDDockWidgets::FrontendType::QtQuick);
    QQmlApplicationEngine engine;
    KDDockWidgets::QtQuick::Platform::instance()->setQmlEngine(&engine);

    // The test server, only when a test asks for it by naming its port.
    std::unique_ptr<spix::AnyRpcServer> server;
    std::unique_ptr<spix::QtQmlBot> bot;
    const int port = qEnvironmentVariableIntValue("OFICINA_SPIX_PORT");
    if (port > 0) {
        server = std::make_unique<spix::AnyRpcServer>(port);
        bot = std::make_unique<spix::QtQmlBot>();
        bot->runTestServer(*server);
    }

    // The typing hook the tests use (keys.h); present only when a test asks for the test server.
    OficinaKeys keys;
    if (port > 0)
        engine.rootContext()->setContextProperty(QStringLiteral("oficinaKeys"), &keys);

    engine.rootContext()->setContextProperty(QStringLiteral("oficinaDrawing"), QVariantMap{{QStringLiteral("adapter"), drawing.adapter}, {QStringLiteral("software"), drawing.software}});

    engine.loadFromModule("Oficina", "Main");
    if (engine.rootObjects().isEmpty())
        return 1;
    return app.exec();
}
