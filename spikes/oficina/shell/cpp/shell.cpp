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

#ifdef Q_OS_WIN
#include <dxgi1_2.h>
#include <wrl/client.h>
#endif

namespace {

// What will draw the windows, asked before the first one (the spike's point 6): the adapter of
// the graphics API Qt Quick draws through here. On Windows that is Direct3D 11 on the adapter Qt's
// RHI takes (the first DXGI lists, or the one `QT_D3D_ADAPTER_INDEX` names), unless
// `QSG_RHI_BACKEND` asks for OpenGL; elsewhere it is OpenGL, through a context made as Qt Quick
// makes its own. A platform with no OpenGL leaves Qt Quick to draw in software itself.
struct Drawing {
    QString adapter;
    bool software;
};

// Renderers that are a processor drawing, not a GPU: Mesa's, Chromium's, Windows' own.
constexpr const char *SOFTWARE[] = {"llvmpipe", "softpipe", "swrast", "SwiftShader", "GDI Generic", "Microsoft Basic Render", "WARP"};

bool namedSoftware(const QString &adapter)
{
    bool software = adapter.isEmpty();
    for (const char *name : SOFTWARE)
        software = software || adapter.contains(QLatin1String(name), Qt::CaseInsensitive);
    return software;
}

Drawing probeOpenGL()
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
    return {adapter, namedSoftware(adapter)};
}

#ifdef Q_OS_WIN
// The adapter Direct3D 11 draws on as Qt's RHI chooses it. WARP, Windows' rasteriser, is a
// software adapter by its flag, and by its ids (Microsoft's vendor 0x1414, device 0x8C: the
// Microsoft Basic Render Driver) where a flag is not set.
Drawing probeDirect3D11()
{
    Microsoft::WRL::ComPtr<IDXGIFactory1> factory;
    if (FAILED(CreateDXGIFactory1(IID_PPV_ARGS(&factory))))
        return {QStringLiteral("no DXGI factory"), true};
    const UINT wanted = qEnvironmentVariableIsSet("QT_D3D_ADAPTER_INDEX") ? UINT(qEnvironmentVariableIntValue("QT_D3D_ADAPTER_INDEX")) : 0;
    Microsoft::WRL::ComPtr<IDXGIAdapter1> adapter;
    if (FAILED(factory->EnumAdapters1(wanted, &adapter)))
        return {QStringLiteral("no Direct3D adapter"), true};
    DXGI_ADAPTER_DESC1 desc = {};
    if (FAILED(adapter->GetDesc1(&desc)))
        return {QStringLiteral("an adapter with no description"), true};
    const QString name = QString::fromWCharArray(desc.Description);
    const bool warp = (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE) != 0 || (desc.VendorId == 0x1414 && desc.DeviceId == 0x8C);
    return {name, warp || namedSoftware(name)};
}
#endif

Drawing probeDrawing()
{
#ifdef Q_OS_WIN
    if (qEnvironmentVariable("QSG_RHI_BACKEND").compare(QLatin1String("opengl"), Qt::CaseInsensitive) != 0)
        return probeDirect3D11();
#endif
    return probeOpenGL();
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
