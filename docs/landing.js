/* --- i18n: same 8 locales as the app, English default --- */

var LOCALES = [
  { code: "en", name: "English" },
  { code: "fr", name: "Français" },
  { code: "es", name: "Español" },
  { code: "de", name: "Deutsch" },
  { code: "pt", name: "Português" },
  { code: "it", name: "Italiano" },
  { code: "zh-CN", name: "简体中文" },
  { code: "ja", name: "日本語" },
];
var DEFAULT_LOCALE = "en";
var LANG_KEY = "depradar_lang";

var en = {
  "landing.tagline": "Detects outdated Rust, JavaScript, Python, Go, PHP, Ruby, and Java dependencies — locally or on a GitHub repository.",
  "landing.ecosystems": "Rust · JavaScript · Python · Go · PHP · Ruby · Java",
  "landing.installBtn": "Install",
  "landing.githubBtn": "View on GitHub",
  "landing.install.title": "Install",
  "landing.install.lead": "One command for your platform, or pick a package below.",
  "landing.tab.macLinux": "macOS / Linux",
  "landing.tab.windows": "Windows",
  "landing.install.mac.title": "macOS (Homebrew)",
  "landing.install.mac.dmgLink": "Or download the .dmg directly →",
  "landing.install.deb.title": "Debian / Ubuntu",
  "landing.install.deb.link": "Download the .deb →",
  "landing.install.appimage.title": "Linux (AppImage)",
  "landing.install.appimage.link": "Download the AppImage →",
  "landing.install.windows.title": "Windows",
  "landing.install.windows.cmd": "Run the installer, then follow the setup wizard",
  "landing.install.windows.link": "Download the .exe →",
  "landing.uninstall.title": "Uninstall",
  "landing.uninstall.lead": "One command for your platform, or remove it manually.",
  "landing.uninstall.mac.title": "macOS",
  "landing.uninstall.mac.note": "Installed by dragging the .dmg instead? Move DepRadar.app to the Trash.",
  "landing.uninstall.deb.title": "Debian / Ubuntu",
  "landing.uninstall.appimage.title": "Linux (AppImage)",
  "landing.uninstall.appimage.note": "(or wherever you placed the AppImage file). To also remove your saved projects, delete ~/.local/share/com.sikander.depradar — or run the uninstall command above.",
  "landing.uninstall.windows.title": "Windows",
  "landing.uninstall.windows.cmd": "Settings → Apps → DepRadar → Uninstall",
  "landing.footer.source": "Source",
  "landing.footer.releases": "Releases",
  "landing.footer.tap": "Homebrew tap",
  "landing.copyLabel": "Copy",
};

var STRINGS = {
  en: en,
  fr: {
    "landing.tagline": "Détecte les dépendances Rust, JavaScript, Python, Go, PHP, Ruby et Java obsolètes — en local ou sur un dépôt GitHub.",
    "landing.ecosystems": "Rust · JavaScript · Python · Go · PHP · Ruby · Java",
    "landing.installBtn": "Installer",
    "landing.githubBtn": "Voir sur GitHub",
    "landing.install.title": "Installation",
    "landing.install.lead": "Une seule commande pour votre plateforme, ou choisissez un paquet ci-dessous.",
    "landing.tab.macLinux": "macOS / Linux",
    "landing.tab.windows": "Windows",
    "landing.install.mac.title": "macOS (Homebrew)",
    "landing.install.mac.dmgLink": "Ou téléchargez le .dmg directement →",
    "landing.install.deb.title": "Debian / Ubuntu",
    "landing.install.deb.link": "Télécharger le .deb →",
    "landing.install.appimage.title": "Linux (AppImage)",
    "landing.install.appimage.link": "Télécharger l'AppImage →",
    "landing.install.windows.title": "Windows",
    "landing.install.windows.cmd": "Lancez l'installeur, puis suivez l'assistant",
    "landing.install.windows.link": "Télécharger le .exe →",
    "landing.uninstall.title": "Désinstallation",
    "landing.uninstall.lead": "Une seule commande pour votre plateforme, ou retirez l'application manuellement.",
    "landing.uninstall.mac.title": "macOS",
    "landing.uninstall.mac.note": "Installé en glissant le .dmg ? Déplacez DepRadar.app vers la Corbeille.",
    "landing.uninstall.deb.title": "Debian / Ubuntu",
    "landing.uninstall.appimage.title": "Linux (AppImage)",
    "landing.uninstall.appimage.note": "(ou où que vous ayez placé le fichier AppImage). Pour supprimer aussi vos projets enregistrés, effacez ~/.local/share/com.sikander.depradar — ou lancez la commande de désinstallation ci-dessus.",
    "landing.uninstall.windows.title": "Windows",
    "landing.uninstall.windows.cmd": "Paramètres → Applications → DepRadar → Désinstaller",
    "landing.footer.source": "Code source",
    "landing.footer.releases": "Releases",
    "landing.footer.tap": "Tap Homebrew",
    "landing.copyLabel": "Copier",
  },
  es: {
    "landing.tagline": "Detecta dependencias obsoletas de Rust, JavaScript, Python, Go, PHP, Ruby y Java, en local o en un repositorio de GitHub.",
    "landing.ecosystems": "Rust · JavaScript · Python · Go · PHP · Ruby · Java",
    "landing.installBtn": "Instalar",
    "landing.githubBtn": "Ver en GitHub",
    "landing.install.title": "Instalación",
    "landing.install.lead": "Un único comando para tu plataforma, o elige un paquete abajo.",
    "landing.tab.macLinux": "macOS / Linux",
    "landing.tab.windows": "Windows",
    "landing.install.mac.title": "macOS (Homebrew)",
    "landing.install.mac.dmgLink": "O descarga el .dmg directamente →",
    "landing.install.deb.title": "Debian / Ubuntu",
    "landing.install.deb.link": "Descargar el .deb →",
    "landing.install.appimage.title": "Linux (AppImage)",
    "landing.install.appimage.link": "Descargar el AppImage →",
    "landing.install.windows.title": "Windows",
    "landing.install.windows.cmd": "Ejecuta el instalador y sigue el asistente",
    "landing.install.windows.link": "Descargar el .exe →",
    "landing.uninstall.title": "Desinstalación",
    "landing.uninstall.lead": "Un único comando para tu plataforma, o elimínalo manualmente.",
    "landing.uninstall.mac.title": "macOS",
    "landing.uninstall.mac.note": "¿Lo instalaste arrastrando el .dmg? Mueve DepRadar.app a la Papelera.",
    "landing.uninstall.deb.title": "Debian / Ubuntu",
    "landing.uninstall.appimage.title": "Linux (AppImage)",
    "landing.uninstall.appimage.note": "(o donde hayas colocado el archivo AppImage). Para borrar también tus proyectos guardados, elimina ~/.local/share/com.sikander.depradar — o ejecuta el comando de desinstalación de arriba.",
    "landing.uninstall.windows.title": "Windows",
    "landing.uninstall.windows.cmd": "Configuración → Aplicaciones → DepRadar → Desinstalar",
    "landing.footer.source": "Código fuente",
    "landing.footer.releases": "Releases",
    "landing.footer.tap": "Tap de Homebrew",
    "landing.copyLabel": "Copiar",
  },
  de: {
    "landing.tagline": "Erkennt veraltete Rust-, JavaScript-, Python-, Go-, PHP-, Ruby- und Java-Abhängigkeiten — lokal oder in einem GitHub-Repository.",
    "landing.ecosystems": "Rust · JavaScript · Python · Go · PHP · Ruby · Java",
    "landing.installBtn": "Installieren",
    "landing.githubBtn": "Auf GitHub ansehen",
    "landing.install.title": "Installation",
    "landing.install.lead": "Ein Befehl für deine Plattform, oder wähle unten ein Paket.",
    "landing.tab.macLinux": "macOS / Linux",
    "landing.tab.windows": "Windows",
    "landing.install.mac.title": "macOS (Homebrew)",
    "landing.install.mac.dmgLink": "Oder lade die .dmg direkt herunter →",
    "landing.install.deb.title": "Debian / Ubuntu",
    "landing.install.deb.link": "Die .deb herunterladen →",
    "landing.install.appimage.title": "Linux (AppImage)",
    "landing.install.appimage.link": "Das AppImage herunterladen →",
    "landing.install.windows.title": "Windows",
    "landing.install.windows.cmd": "Installer ausführen und dem Assistenten folgen",
    "landing.install.windows.link": "Die .exe herunterladen →",
    "landing.uninstall.title": "Deinstallation",
    "landing.uninstall.lead": "Ein Befehl für deine Plattform, oder entferne die App manuell.",
    "landing.uninstall.mac.title": "macOS",
    "landing.uninstall.mac.note": "Per .dmg installiert? Verschiebe DepRadar.app in den Papierkorb.",
    "landing.uninstall.deb.title": "Debian / Ubuntu",
    "landing.uninstall.appimage.title": "Linux (AppImage)",
    "landing.uninstall.appimage.note": "(oder wo auch immer die AppImage-Datei liegt). Um auch Ihre gespeicherten Projekte zu entfernen, löschen Sie ~/.local/share/com.sikander.depradar — oder führen Sie den obigen Deinstallationsbefehl aus.",
    "landing.uninstall.windows.title": "Windows",
    "landing.uninstall.windows.cmd": "Einstellungen → Apps → DepRadar → Deinstallieren",
    "landing.footer.source": "Quellcode",
    "landing.footer.releases": "Releases",
    "landing.footer.tap": "Homebrew-Tap",
    "landing.copyLabel": "Kopieren",
  },
  pt: {
    "landing.tagline": "Deteta dependências Rust, JavaScript, Python, Go, PHP, Ruby e Java desatualizadas — localmente ou num repositório GitHub.",
    "landing.ecosystems": "Rust · JavaScript · Python · Go · PHP · Ruby · Java",
    "landing.installBtn": "Instalar",
    "landing.githubBtn": "Ver no GitHub",
    "landing.install.title": "Instalação",
    "landing.install.lead": "Um único comando para a sua plataforma, ou escolha um pacote abaixo.",
    "landing.tab.macLinux": "macOS / Linux",
    "landing.tab.windows": "Windows",
    "landing.install.mac.title": "macOS (Homebrew)",
    "landing.install.mac.dmgLink": "Ou descarregue o .dmg diretamente →",
    "landing.install.deb.title": "Debian / Ubuntu",
    "landing.install.deb.link": "Descarregar o .deb →",
    "landing.install.appimage.title": "Linux (AppImage)",
    "landing.install.appimage.link": "Descarregar o AppImage →",
    "landing.install.windows.title": "Windows",
    "landing.install.windows.cmd": "Execute o instalador e siga o assistente",
    "landing.install.windows.link": "Descarregar o .exe →",
    "landing.uninstall.title": "Desinstalação",
    "landing.uninstall.lead": "Um único comando para a sua plataforma, ou remova-o manualmente.",
    "landing.uninstall.mac.title": "macOS",
    "landing.uninstall.mac.note": "Instalado arrastando o .dmg? Mova o DepRadar.app para o Lixo.",
    "landing.uninstall.deb.title": "Debian / Ubuntu",
    "landing.uninstall.appimage.title": "Linux (AppImage)",
    "landing.uninstall.appimage.note": "(ou onde tiver colocado o ficheiro AppImage). Para apagar também os seus projetos guardados, elimine ~/.local/share/com.sikander.depradar — ou execute o comando de desinstalação acima.",
    "landing.uninstall.windows.title": "Windows",
    "landing.uninstall.windows.cmd": "Definições → Aplicações → DepRadar → Desinstalar",
    "landing.footer.source": "Código-fonte",
    "landing.footer.releases": "Releases",
    "landing.footer.tap": "Tap do Homebrew",
    "landing.copyLabel": "Copiar",
  },
  it: {
    "landing.tagline": "Rileva le dipendenze obsolete di Rust, JavaScript, Python, Go, PHP, Ruby e Java — in locale o su un repository GitHub.",
    "landing.ecosystems": "Rust · JavaScript · Python · Go · PHP · Ruby · Java",
    "landing.installBtn": "Installa",
    "landing.githubBtn": "Vedi su GitHub",
    "landing.install.title": "Installazione",
    "landing.install.lead": "Un solo comando per la tua piattaforma, oppure scegli un pacchetto qui sotto.",
    "landing.tab.macLinux": "macOS / Linux",
    "landing.tab.windows": "Windows",
    "landing.install.mac.title": "macOS (Homebrew)",
    "landing.install.mac.dmgLink": "Oppure scarica il .dmg direttamente →",
    "landing.install.deb.title": "Debian / Ubuntu",
    "landing.install.deb.link": "Scarica il .deb →",
    "landing.install.appimage.title": "Linux (AppImage)",
    "landing.install.appimage.link": "Scarica l'AppImage →",
    "landing.install.windows.title": "Windows",
    "landing.install.windows.cmd": "Avvia l'installer e segui la procedura guidata",
    "landing.install.windows.link": "Scarica il .exe →",
    "landing.uninstall.title": "Disinstallazione",
    "landing.uninstall.lead": "Un solo comando per la tua piattaforma, oppure rimuovila manualmente.",
    "landing.uninstall.mac.title": "macOS",
    "landing.uninstall.mac.note": "Installato trascinando il .dmg? Sposta DepRadar.app nel Cestino.",
    "landing.uninstall.deb.title": "Debian / Ubuntu",
    "landing.uninstall.appimage.title": "Linux (AppImage)",
    "landing.uninstall.appimage.note": "(o dovunque tu abbia messo il file AppImage). Per eliminare anche i progetti salvati, cancella ~/.local/share/com.sikander.depradar — oppure esegui il comando di disinstallazione qui sopra.",
    "landing.uninstall.windows.title": "Windows",
    "landing.uninstall.windows.cmd": "Impostazioni → App → DepRadar → Disinstalla",
    "landing.footer.source": "Codice sorgente",
    "landing.footer.releases": "Release",
    "landing.footer.tap": "Tap Homebrew",
    "landing.copyLabel": "Copia",
  },
  "zh-CN": {
    "landing.tagline": "检测 Rust、JavaScript、Python、Go、PHP、Ruby 和 Java 项目中过时的依赖 —— 支持本地项目或 GitHub 仓库。",
    "landing.ecosystems": "Rust · JavaScript · Python · Go · PHP · Ruby · Java",
    "landing.installBtn": "安装",
    "landing.githubBtn": "在 GitHub 上查看",
    "landing.install.title": "安装",
    "landing.install.lead": "一条适合你平台的命令，或在下方选择安装包。",
    "landing.tab.macLinux": "macOS / Linux",
    "landing.tab.windows": "Windows",
    "landing.install.mac.title": "macOS（Homebrew）",
    "landing.install.mac.dmgLink": "或直接下载 .dmg →",
    "landing.install.deb.title": "Debian / Ubuntu",
    "landing.install.deb.link": "下载 .deb →",
    "landing.install.appimage.title": "Linux（AppImage）",
    "landing.install.appimage.link": "下载 AppImage →",
    "landing.install.windows.title": "Windows",
    "landing.install.windows.cmd": "运行安装程序，然后按照向导完成安装",
    "landing.install.windows.link": "下载 .exe →",
    "landing.uninstall.title": "卸载",
    "landing.uninstall.lead": "一条适合你平台的命令，或手动移除。",
    "landing.uninstall.mac.title": "macOS",
    "landing.uninstall.mac.note": "通过拖动 .dmg 安装的？将 DepRadar.app 移到废纸篓即可。",
    "landing.uninstall.deb.title": "Debian / Ubuntu",
    "landing.uninstall.appimage.title": "Linux（AppImage）",
    "landing.uninstall.appimage.note": "（或你存放 AppImage 文件的任何位置）。如需同时删除已保存的项目，请删除 ~/.local/share/com.sikander.depradar —— 或运行上面的卸载命令。",
    "landing.uninstall.windows.title": "Windows",
    "landing.uninstall.windows.cmd": "设置 → 应用 → DepRadar → 卸载",
    "landing.footer.source": "源代码",
    "landing.footer.releases": "发布版本",
    "landing.footer.tap": "Homebrew Tap",
    "landing.copyLabel": "复制",
  },
  ja: {
    "landing.tagline": "Rust、JavaScript、Python、Go、PHP、Ruby、Java の古くなった依存関係を検出します —— ローカルまたは GitHub リポジトリで。",
    "landing.ecosystems": "Rust · JavaScript · Python · Go · PHP · Ruby · Java",
    "landing.installBtn": "インストール",
    "landing.githubBtn": "GitHub で見る",
    "landing.install.title": "インストール",
    "landing.install.lead": "お使いのプラットフォーム用のコマンド一行、または下からパッケージを選択してください。",
    "landing.tab.macLinux": "macOS / Linux",
    "landing.tab.windows": "Windows",
    "landing.install.mac.title": "macOS（Homebrew）",
    "landing.install.mac.dmgLink": "または .dmg を直接ダウンロード →",
    "landing.install.deb.title": "Debian / Ubuntu",
    "landing.install.deb.link": ".deb をダウンロード →",
    "landing.install.appimage.title": "Linux（AppImage）",
    "landing.install.appimage.link": "AppImage をダウンロード →",
    "landing.install.windows.title": "Windows",
    "landing.install.windows.cmd": "インストーラーを実行し、ウィザードに従ってください",
    "landing.install.windows.link": ".exe をダウンロード →",
    "landing.uninstall.title": "アンインストール",
    "landing.uninstall.lead": "お使いのプラットフォーム用のコマンド一行、または手動で削除してください。",
    "landing.uninstall.mac.title": "macOS",
    "landing.uninstall.mac.note": ".dmg をドラッグしてインストールした場合は、DepRadar.app をゴミ箱に移動してください。",
    "landing.uninstall.deb.title": "Debian / Ubuntu",
    "landing.uninstall.appimage.title": "Linux（AppImage）",
    "landing.uninstall.appimage.note": "（AppImage ファイルを置いた場所ならどこでも構いません）。保存したプロジェクトも削除するには ~/.local/share/com.sikander.depradar を削除するか、上のアンインストールコマンドを実行してください。",
    "landing.uninstall.windows.title": "Windows",
    "landing.uninstall.windows.cmd": "設定 → アプリ → DepRadar → アンインストール",
    "landing.footer.source": "ソースコード",
    "landing.footer.releases": "リリース",
    "landing.footer.tap": "Homebrew タップ",
    "landing.copyLabel": "コピー",
  },
};

function getLocale() {
  try {
    var stored = localStorage.getItem(LANG_KEY);
    if (stored && LOCALES.some(function (l) { return l.code === stored; })) return stored;
  } catch {
    /* localStorage unavailable — fall through to the default */
  }
  return DEFAULT_LOCALE;
}

function setLocale(code) {
  if (!LOCALES.some(function (l) { return l.code === code; })) return;
  try {
    localStorage.setItem(LANG_KEY, code);
  } catch {
    /* ignore persistence failures, the locale still applies now */
  }
  document.documentElement.lang = code;
  applyTranslations();
}

function t(key) {
  var dict = STRINGS[getLocale()] || en;
  return dict[key] || en[key] || key;
}

function applyTranslations() {
  document.querySelectorAll("[data-i18n]").forEach(function (el) {
    el.textContent = t(el.dataset.i18n);
  });
  document.querySelectorAll("[data-i18n-title]").forEach(function (el) {
    var label = t(el.dataset.i18nTitle);
    el.title = label;
    el.setAttribute("aria-label", label);
  });
}

/* --- language switcher --- */

var select = document.getElementById("lang-select");
LOCALES.forEach(function (locale) {
  var opt = document.createElement("option");
  opt.value = locale.code;
  opt.textContent = locale.name;
  select.appendChild(opt);
});
select.value = getLocale();
select.addEventListener("change", function () {
  setLocale(select.value);
});

applyTranslations();

/* --- platform tabs --- */

function initTabs(rowId, panelPrefix) {
  var row = document.getElementById(rowId);
  if (!row) return;
  row.querySelectorAll(".tab-btn").forEach(function (btn) {
    btn.addEventListener("click", function () {
      row.querySelectorAll(".tab-btn").forEach(function (b) {
        b.classList.toggle("active", b === btn);
      });
      ["mac", "win"].forEach(function (os) {
        var panel = document.getElementById(panelPrefix + "-" + os);
        if (panel) panel.classList.toggle("hidden", os !== btn.dataset.os);
      });
    });
  });
}
initTabs("install-tabs", "install");
initTabs("uninstall-tabs", "uninstall");

/* --- copy-to-clipboard buttons --- */

function fallbackCopy(text) {
  var ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  try {
    document.execCommand("copy");
  } catch {
    /* nothing more we can do */
  }
  document.body.removeChild(ta);
}

function initCopyButtons() {
  document.querySelectorAll(".copy-btn").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var target = document.getElementById(btn.dataset.copy);
      if (!target) return;
      var text = target.textContent.trim();
      var showCopied = function () {
        var original = btn.textContent;
        btn.textContent = "✓";
        btn.classList.add("copied");
        setTimeout(function () {
          btn.textContent = original;
          btn.classList.remove("copied");
        }, 1500);
      };
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard
          .writeText(text)
          .then(showCopied)
          .catch(function () {
            fallbackCopy(text);
            showCopied();
          });
      } else {
        fallbackCopy(text);
        showCopied();
      }
    });
  });
}
initCopyButtons();

/* --- direct download links ---
 * Their filenames embed the version, so they can't be hardcoded —
 * resolve them against the latest release once, on load. Falls back
 * to the releases page (already the href) if the API call fails or
 * rate-limits. A suffix must match the END of the file name, so that
 * "x.deb" never picks up its "x.deb.sig" signature. */

function pickAsset(assets, suffix) {
  return (assets || []).find(function (a) {
    return a.name.endsWith(suffix);
  });
}

fetch("https://api.github.com/repos/depradar-app/depradar/releases/latest")
  .then(function (r) {
    return r.json();
  })
  .then(function (release) {
    document.querySelectorAll("a.direct[data-asset]").forEach(function (link) {
      var asset = pickAsset(release.assets, link.dataset.asset);
      if (asset) link.href = asset.browser_download_url;
    });
  })
  .catch(function () {
    /* keep the releases-page fallback href */
  });
