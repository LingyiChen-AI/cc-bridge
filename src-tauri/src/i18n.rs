//! 后端文案（托盘菜单、系统通知）的多语言。界面文案在前端 src/locales/。
//! 语言代码与前端一致：zh, zh-TW, en, ja, ko, es, fr, de, ru, pt。

pub const LANGS: [&str; 10] = ["zh", "zh-TW", "en", "ja", "ko", "es", "fr", "de", "ru", "pt"];

/// 系统 / 用户给的语言标签 → 支持的语言代码；不支持的回落到 en。
pub fn normalize(tag: &str) -> &'static str {
    let t = tag.trim().replace('_', "-").to_lowercase();
    if t.starts_with("zh") {
        return if ["tw", "hk", "mo", "hant"].iter().any(|k| t.contains(k)) { "zh-TW" } else { "zh" };
    }
    let base = t.split('-').next().unwrap_or("");
    LANGS.iter().copied().find(|l| *l == base).unwrap_or("en")
}

// 每行顺序与 LANGS 相同
type Row = [&'static str; 10];

const TABLE: &[(&str, Row)] = &[
    ("trayProxyRunning", ["代理运行中", "代理執行中", "Proxy running", "プロキシ稼働中", "프록시 실행 중", "Proxy en marcha", "Proxy en marche", "Proxy läuft", "Прокси работает", "Proxy em execução"]),
    ("trayProxyStarting", ["代理启动中", "代理啟動中", "Proxy starting", "プロキシ起動中", "프록시 시작 중", "Iniciando proxy", "Démarrage du proxy", "Proxy startet", "Запуск прокси", "Iniciando proxy"]),
    ("trayProxyRestarting", ["代理重启中", "代理重新啟動中", "Proxy restarting", "プロキシ再起動中", "프록시 다시 시작 중", "Reiniciando proxy", "Redémarrage du proxy", "Proxy wird neu gestartet", "Перезапуск прокси", "Reiniciando proxy"]),
    ("trayProxyFailed", ["代理已停止：反复崩溃", "代理已停止：反覆當機", "Proxy stopped: crashed repeatedly", "プロキシ停止：クラッシュを繰り返しました", "프록시 중지: 반복 종료", "Proxy detenido: fallos repetidos", "Proxy arrêté : plantages répétés", "Proxy gestoppt: wiederholte Abstürze", "Прокси остановлен: повторные сбои", "Proxy parado: falhas repetidas"]),
    ("trayPortBusy", ["端口被占用", "連接埠已被占用", "Port in use", "ポート使用中", "포트 사용 중", "Puerto en uso", "Port occupé", "Port belegt", "Порт занят", "Porta em uso"]),
    ("trayProxyStopped", ["代理未启动", "代理未啟動", "Proxy not started", "プロキシ未起動", "프록시 시작 안 됨", "Proxy sin iniciar", "Proxy non démarré", "Proxy nicht gestartet", "Прокси не запущен", "Proxy não iniciado"]),
    ("trayAuth401", ["Codex 登录无效（401）", "Codex 登入無效（401）", "Codex login invalid (401)", "Codex ログイン無効（401）", "Codex 로그인 무효(401)", "Sesión de Codex no válida (401)", "Connexion Codex invalide (401)", "Codex-Anmeldung ungültig (401)", "Вход в Codex недействителен (401)", "Login do Codex inválido (401)"]),
    ("trayAuthOk", ["Codex 已登录", "Codex 已登入", "Codex signed in", "Codex ログイン済み", "Codex 로그인됨", "Sesión de Codex iniciada", "Codex connecté", "Codex angemeldet", "Вход в Codex выполнен", "Codex conectado"]),
    ("trayAuthRefresh", ["Codex 令牌待续期", "Codex 權杖待續期", "Codex token pending refresh", "Codex トークン更新待ち", "Codex 토큰 갱신 대기", "Token de Codex pendiente de renovar", "Jeton Codex à renouveler", "Codex-Token wird erneuert", "Токен Codex ожидает обновления", "Token do Codex aguardando renovação"]),
    ("trayAuthMissing", ["Codex 未登录", "Codex 未登入", "Codex not signed in", "Codex 未ログイン", "Codex 로그인 안 됨", "Codex sin sesión", "Codex non connecté", "Codex nicht angemeldet", "Вход в Codex не выполнен", "Codex não conectado"]),
    ("trayTaken", ["Claude Code 已接管", "Claude Code 已接管", "Claude Code connected", "Claude Code 接続済み", "Claude Code 연결됨", "Claude Code conectado", "Claude Code connecté", "Claude Code verbunden", "Claude Code подключён", "Claude Code conectado"]),
    ("trayWaiting", ["Claude Code 等待接管", "Claude Code 等待接管", "Claude Code waiting", "Claude Code 接続待ち", "Claude Code 연결 대기", "Claude Code en espera", "Claude Code en attente", "Claude Code wartet", "Claude Code ожидает", "Claude Code aguardando"]),
    ("trayNotTaken", ["Claude Code 未接管", "Claude Code 未接管", "Claude Code not connected", "Claude Code 未接続", "Claude Code 연결 안 됨", "Claude Code sin conectar", "Claude Code non connecté", "Claude Code nicht verbunden", "Claude Code не подключён", "Claude Code não conectado"]),
    ("menuOpen", ["打开主窗口", "開啟主視窗", "Open window", "ウィンドウを開く", "창 열기", "Abrir ventana", "Ouvrir la fenêtre", "Fenster öffnen", "Открыть окно", "Abrir janela"]),
    ("menuTakeover", ["接管 Claude Code", "接管 Claude Code", "Connect Claude Code", "Claude Code に接続", "Claude Code 연결", "Conectar Claude Code", "Connecter Claude Code", "Claude Code verbinden", "Подключить Claude Code", "Conectar Claude Code"]),
    ("menuRestart", ["重启代理", "重新啟動代理", "Restart proxy", "プロキシを再起動", "프록시 다시 시작", "Reiniciar proxy", "Redémarrer le proxy", "Proxy neu starten", "Перезапустить прокси", "Reiniciar proxy"]),
    ("menuAutostart", ["开机自启", "開機自動啟動", "Launch at login", "ログイン時に起動", "로그인 시 실행", "Abrir al iniciar sesión", "Lancer à l’ouverture de session", "Bei Anmeldung starten", "Запускать при входе", "Abrir ao iniciar a sessão"]),
    ("menuQuit", ["退出", "結束", "Quit", "終了", "종료", "Salir", "Quitter", "Beenden", "Выход", "Sair"]),
    ("notifyFailedTitle", ["CC Bridge：代理不可用", "CC Bridge：代理無法使用", "CC Bridge: proxy unavailable", "CC Bridge：プロキシを利用できません", "CC Bridge: 프록시 사용 불가", "CC Bridge: proxy no disponible", "CC Bridge : proxy indisponible", "CC Bridge: Proxy nicht verfügbar", "CC Bridge: прокси недоступен", "CC Bridge: proxy indisponível"]),
    ("notifyFailedBody", [
        "已临时撤销对 Claude Code 的接管，新开的 Claude Code 窗口只用 Claude 模型。",
        "已暫時取消對 Claude Code 的接管，新開的 Claude Code 視窗只使用 Claude 模型。",
        "Claude Code has been disconnected for now; new Claude Code windows will use Claude models only.",
        "Claude Code との接続を一時的に解除しました。新しい Claude Code ウィンドウは Claude モデルのみ使用します。",
        "Claude Code 연결을 일시적으로 해제했습니다. 새 Claude Code 창은 Claude 모델만 사용합니다.",
        "Claude Code se ha desconectado por ahora; las nuevas ventanas usarán solo modelos de Claude.",
        "Claude Code est déconnecté pour le moment ; les nouvelles fenêtres n’utiliseront que les modèles Claude.",
        "Claude Code wurde vorerst getrennt; neue Claude-Code-Fenster nutzen nur Claude-Modelle.",
        "Claude Code временно отключён; новые окна Claude Code будут использовать только модели Claude.",
        "O Claude Code foi desconectado por enquanto; novas janelas usarão apenas modelos Claude.",
    ]),
];

pub fn tr(lang: &str, key: &str) -> &'static str {
    let idx = LANGS.iter().position(|l| *l == lang).unwrap_or(2);
    TABLE.iter().find(|(k, _)| *k == key).map(|(_, row)| row[idx]).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_translated_in_every_language() {
        for (key, row) in TABLE {
            for (i, text) in row.iter().enumerate() {
                assert!(!text.is_empty(), "{key} 缺少 {}", LANGS[i]);
            }
        }
        assert_eq!(tr("fr", "menuQuit"), "Quitter");
        assert_eq!(tr("xx", "menuQuit"), "Quit");
    }

    #[test]
    fn normalize_system_locales() {
        for (input, want) in [("zh-CN", "zh"), ("zh_Hans_CN", "zh"), ("zh-TW", "zh-TW"), ("zh-Hant-HK", "zh-TW"), ("ja-JP", "ja"), ("pt-BR", "pt"), ("nl-NL", "en"), ("", "en")] {
            assert_eq!(normalize(input), want, "{input}");
        }
    }
}
