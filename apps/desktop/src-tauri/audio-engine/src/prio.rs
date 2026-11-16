//! Prioridade do áudio no Windows.
//!
//! O cpal já deixa a thread do callback com prioridade máxima (`TIME_CRITICAL`), mas isso não
//! basta: sem registrar a thread no MMCSS (o agendador multimídia do Windows, perfil "Pro Audio")
//! o sistema pode atrasar o callback quando outro programa usa a CPU, e com buffers pequenos
//! isso vira estalo. Aqui também desligamos a "economia de energia" do processo (o modo de
//! eficiência do Windows 11 reduz a CPU de programas que ele acha que estão em segundo plano).
//!
//! Tudo é "melhor esforço": se uma chamada falhar, o áudio segue normalmente e o motivo vai para
//! o log. Em outros sistemas as funções não fazem nada.

/// Prioridade do processo e fim da economia de energia. Chamar uma vez, ao iniciar o motor.
pub fn boost_process() {
    #[cfg(windows)]
    {
        log::info!("prioridade do processo: {}", win::boost_process());
    }
}

/// Registra a thread atual no MMCSS como "Pro Audio". Chamar uma vez de dentro do callback.
pub fn boost_audio_thread(what: &str) {
    #[cfg(windows)]
    {
        log::info!("prioridade do áudio ({what}): {}", win::boost_thread());
    }
    #[cfg(not(windows))]
    {
        let _ = what;
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    #[link(name = "avrt")]
    extern "system" {
        fn AvSetMmThreadCharacteristicsW(
            task_name: *const u16,
            task_index: *mut u32,
        ) -> *mut c_void;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn SetPriorityClass(process: *mut c_void, class: u32) -> i32;
        fn SetProcessInformation(
            process: *mut c_void,
            class: i32,
            info: *mut c_void,
            size: u32,
        ) -> i32;
        fn GetLastError() -> u32;
    }

    const HIGH_PRIORITY_CLASS: u32 = 0x0000_0080;
    const PROCESS_POWER_THROTTLING: i32 = 4;
    const POWER_THROTTLING_CURRENT_VERSION: u32 = 1;
    const POWER_THROTTLING_EXECUTION_SPEED: u32 = 1;

    #[repr(C)]
    struct PowerThrottlingState {
        version: u32,
        control_mask: u32,
        state_mask: u32,
    }

    pub fn boost_process() -> String {
        // SAFETY: chamadas simples da API do Windows sobre o processo atual.
        unsafe {
            let process = GetCurrentProcess();
            let class = if SetPriorityClass(process, HIGH_PRIORITY_CLASS) != 0 {
                "classe alta".to_string()
            } else {
                format!("classe alta falhou (erro {})", GetLastError())
            };
            // ControlMask = quero controlar; StateMask = 0 → economia de energia DESLIGADA.
            let mut state = PowerThrottlingState {
                version: POWER_THROTTLING_CURRENT_VERSION,
                control_mask: POWER_THROTTLING_EXECUTION_SPEED,
                state_mask: 0,
            };
            let throttling = if SetProcessInformation(
                process,
                PROCESS_POWER_THROTTLING,
                (&mut state as *mut PowerThrottlingState).cast(),
                std::mem::size_of::<PowerThrottlingState>() as u32,
            ) != 0
            {
                "sem economia de energia".to_string()
            } else {
                format!("economia de energia não alterada (erro {})", GetLastError())
            };
            format!("{class}, {throttling}")
        }
    }

    pub fn boost_thread() -> String {
        let name: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
        let mut index = 0u32;
        // SAFETY: `name` é uma string UTF-16 terminada em zero que vive durante a chamada.
        let handle = unsafe { AvSetMmThreadCharacteristicsW(name.as_ptr(), &mut index) };
        if handle.is_null() {
            // SAFETY: só lê o último erro da thread.
            format!("MMCSS falhou (erro {})", unsafe { GetLastError() })
        } else {
            format!("MMCSS \"Pro Audio\" ativo (tarefa {index})")
        }
    }
}
