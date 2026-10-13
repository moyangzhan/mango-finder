use crate::enums::FileContentLanguage;
use crate::utils::app_util::get_default_file_content_language;
use anyhow::Result;
use log::info;
use rusqlite::Connection;
use uuid::Uuid;

/// 使用传入的连接执行 DDL
pub fn exec_ddl_with_conn(conn: &Connection) -> Result<()> {
    info!("dbv1.exec_ddl_with_conn");
    // config table
    conn.execute_batch(
        r#"create table if not exists config(
            id    integer primary key autoincrement,
            name  varchar(50) default '' unique not null,
            value text        default ''        not null,
            create_time text  default '' not null,
            update_time text  default '' not null
        );
        create trigger if not exists config_create_time 
        after insert on config
        for each row
        begin
            update config 
            set create_time = datetime('now', 'localtime'),
                update_time = datetime('now', 'localtime')
            where id = new.id;
        end;
        create trigger if not exists config_update_time 
        after update on config
        for each row
        begin
            update config set update_time = datetime('now', 'localtime')
            where id = new.id;
        end;
        "#,
    )?;

    // model_platform table
    conn.execute_batch(
        r#"
        create table if not exists model_platform (
            id                          integer     primary key autoincrement,
            name                        text        default ''  unique   not null, -- e.g., openai, deepseek, dashscope, siliconflow
            title                       text        default ''           not null, -- A more readable name, e.g., OpenAi, DeepSeek, 百炼，硅基流动
            logo                        text        default ''           not null, -- logo url or path
            base_url                    text        default ''           not null,
            api_key                     text        default ''           not null,
            remark                      text        default ''           not null,
            is_proxy_enable             integer     default 0            not null,
            is_openai_api_compatible    integer     default 0            not null, -- 0: not compatible, 1: compatible
            office_site_url             text        default ''           not null,
            create_time                 text        default ''           not null,
            update_time                 text        default ''           not null
        );
        create trigger if not exists model_platform_create_time
        after insert on model_platform
        for each row
        begin
            update model_platform
            set create_time = datetime('now', 'localtime'),
                update_time = datetime('now', 'localtime')
            where id = new.id;
        end;
        create trigger if not exists model_platform_update_time
        after update on model_platform
        for each row
        begin
            update model_platform set update_time = datetime('now', 'localtime')
            where id = new.id;
        end;
        "#,
    )?;

    //ai_model table
    conn.execute_batch(
        r#"
            create table if not exists ai_model
            (
                id                   integer       primary key autoincrement,
                name                 varchar(45)   default ''                not null,
                title                varchar(45)   default ''                not null, -- Model title, a more readable name, e.g., openai-gpt3
                model_types           varchar(45)   default 'text'            not null, -- e.g., text, image, vision, embedding, rerank, asr, tts
                setting              text          default ''                not null, -- json format, e.g., {voice_for_group1: "v1", voice_for_group2: "v2"}
                remark               text          default ''                not null,
                platform             varchar(45)   default ''                not null,
                context_window       integer       default 0                 not null,
                max_input_tokens     integer       default 0                 not null,
                max_output_tokens    integer       default 0                 not null,
                input_types          text          default 'text'            not null, -- text, image, audio, video
                output_types         text          default 'text'            not null, -- text, image, audio, video
                properties           text          default '{}'              not null, -- json format, e.g. { "dimension": 1536 } for embedding model,{"voices":["v1","v2"]} for tts
                is_reasoner          integer       default 0                 not null, -- true: Reasoning model like DeepSeek-R1, false: Non-reasoning model like gpt-3.5-turbo
                is_thinking_closable integer       default 0                 not null, -- true: Thinking can be closed, false: Thinking cannot be closed like DeepSeek-R1
                is_free              integer       default 0                 not null, -- true: Free model, false: Paid model
                is_enable            integer       default 0                 not null,
                create_time          text          default '' not null,
                update_time          text          default '' not null,
                unique (name, platform)
            );
            create trigger if not exists ai_model_create_time 
            after insert on ai_model
            for each row
            begin
                update ai_model 
                set create_time = datetime('now', 'localtime'),
                    update_time = datetime('now', 'localtime')
                where id = new.id;
            end;
            create trigger if not exists ai_model_update_time 
            after update on ai_model
            for each row
            begin
                update ai_model set update_time = datetime('now', 'localtime')
                where id = new.id;
            end;
        "#
    )?;

    // File indexing tables
    conn.execute_batch(
        r#"
        create table if not exists file_info(
            id integer primary key autoincrement,
            name text not null default '',
            path text not null default '',
            category integer not null default 0,                -- 1:document, 2:image, 3:audio, 4:video, 5:other
            content text not null default '',                   -- e.g., image recognition content, audio asr text, video reconition content
            metadata text not null default '{}',                -- json format, e.g., { "author": "John", "description": "Sample file" }
            md5 text not null default '',
            file_ext text not null default '',
            file_size integer not null default 0,
            file_create_time text not null default '',
            file_update_time text not null default '',
            is_invalid integer not null default 1,              -- 1: normal, 0: invalid
            invalid_reason text not null default '',            -- e.g., "The file is broken"
            content_index_status integer not null default 1,    -- 1: waiting, 2: indexing, 3: indexed, 4: index failed
            content_index_status_msg text not null default '',  -- e.g., "The file is encrypted"
            meta_index_status integer not null default 1,       -- 1: waiting, 2: indexing, 3: indexed, 4: index failed
            meta_index_status_msg text not null default '',
            create_time text not null default '',
            update_time text not null default ''
        );

        CREATE INDEX IF NOT EXISTS idx_name ON file_info(name);
        CREATE INDEX IF NOT EXISTS idx_path ON file_info(path);
        CREATE INDEX IF NOT EXISTS idx_md5 ON file_info(md5);
        CREATE INDEX IF NOT EXISTS idx_metadata ON file_info(metadata);

        CREATE virtual table IF NOT EXISTS file_content_fts USING fts5(
            file_id UNINDEXED, chunk_id UNINDEXED, content, tokenize='porter'
        );
        create virtual table if not exists file_metadata_vec using vec0(
            id integer primary key autoincrement,
            embedding float[256] distance_metric=cosine -- document metadata embedding, source text from file_info.metadata
        );
        CREATE TABLE IF NOT EXISTS file_metadata_data(
            id INTEGER PRIMARY KEY,                 -- file_metadata_vec.id
            file_id integer default 0 not null,     -- foreign key to file_info.id
            sparse_weights blob                     -- for hybrid search, store sparse vector in binary format
        );
        create virtual table if not exists file_content_vec using vec0(
            id integer primary key autoincrement,
            embedding float[1024] distance_metric=cosine
        );
        create table file_content_data(
            id INTEGER PRIMARY KEY,                   -- file_content_vec.id
            file_id INTEGER default 0 not null,       -- foreign key to file_info.id
            chunk_index INTEGER default 0 not null,
            chunk_text text default '' NOT NULL,
            sparse_weights BLOB NOT NULL              -- for hybrid search, store sparse vector in binary format
        );
        create index if not exists idx_fcd_file_id on file_content_data(file_id);
        
        create trigger if not exists file_info_create_time 
        after insert on file_info
        for each row
        begin
            update file_info 
            set create_time = datetime('now', 'localtime'),
                update_time = datetime('now', 'localtime')
            where id = new.id;
        end;
        create trigger if not exists file_info_update_time 
        after update on file_info
        for each row
        begin
            update file_info set update_time = datetime('now', 'localtime')
            where id = new.id;
        end;
        -- delete related content and metadata when delete a file_info record
        create trigger if not exists trg_delete_file_content_data
        before delete on file_info
        for each row
        begin
            delete from file_content_vec where id in (select id from file_content_data where file_id = old.id);
            delete from file_content_data where file_id = old.id;
        end;
        "#,
    )?;

    conn.execute_batch(
        r#"create table if not exists indexing_task(
            id    integer primary key autoincrement,
            paths  text not null default '',                        -- File paths to be indexed, separated by comma
            embedding_model text not null default '',               -- Embedding model name, e.g., all-minilm-l6-v2
            status text not null default 'pending',                 -- Task status: pending, running, paused, completed, failed, cancelled
            start_time text not null default '',                    -- Empty if not started
            end_time text not null default '',                      -- Empty if not completed
            duration integer not null default 0,                    -- Completion time in seconds
            total_cnt integer not null default 0,                   -- Total number of files to be processed in this task
            content_processed_cnt integer not null default 0,       -- content_indexed_success_cnt + content_indexed_failed_cnt + content_indexed_skipped_cnt
            content_indexed_success_cnt integer not null default 0, -- Successfully indexed contents
            content_indexed_failed_cnt integer not null default 0,  -- Failed contents, including invalid files
            content_indexed_skipped_cnt integer not null default 0, -- Skipped contents. e.g., binary file, encrypted file
            remark text not null default '',                        -- Task remarks
            config_json text not null default '{}',                 -- JSON configuration for indexing task (e.g., filters, rules)
            triggered_by integer DEFAULT 1 not null,                -- 1: file selector, 2: file watcher
            create_time text not null default '',
            update_time text not null default ''
        );
        create trigger if not exists indexing_task_create_time 
        after insert on indexing_task
        for each row
        begin
            update indexing_task 
            set create_time = datetime('now', 'localtime'),
                update_time = datetime('now', 'localtime')
            where id = new.id;
        end;
        create trigger if not exists indexing_task_update_time 
        after update on indexing_task
        for each row
        begin
            update indexing_task set update_time = datetime('now', 'localtime')
            where id = new.id;
        end;
        "#,
    )?;

    Ok(())
}

/// 使用传入的连接初始化数据
pub fn init_data_with_conn(conn: &Connection) -> Result<()> {
    info!("dbv1.init_data_with_conn");

    // init client_id
    let client_id = Uuid::new_v4().to_string().replace("-", "");
    conn.execute(
        "insert or ignore into config (name, value) VALUES (?1, ?2)",
        ("client_id", client_id),
    )?;

    // Global config
    // Proxy for model requests
    conn.execute(
        "insert or ignore into config (name, value) VALUES (?1, ?2)",
        (
            "proxy",
            r#"{"protocol":"http","host":"127.0.0.1","port":1087}"#,
        ),
    )?;

    let (default_locale, default_file_content_language) =
        if get_default_file_content_language() == FileContentLanguage::Chinese {
            ("zh-CN", "zh")
        } else {
            ("en-US", "en")
        };
    conn.execute(
        "insert or ignore into config (name, value) VALUES ('active_locale', ?1)",
        (default_locale,),
    )?;
    let default_indexer_setting = format!(
        r#"{{"is_private":true,"parser_mode":"local","file_content_language":"{}","image_parser_mode":"local","audio_parser_mode":"local","ignore_dirs":["node_modules"],"ignore_path_prefixes":[],"ignore_exts":["tmp"],"ignore_files":[],"save_parsed_content": {{"document":false,"image":true,"audio":true,"video":true}}}}"#,
        default_file_content_language
    );
    // is_private: Deprecated, kept for backward compatibility parser_mode: "local" | "selfhosted" | "remote" | "mixed"
    conn.execute(
        "insert or ignore into config (name, value) VALUES ('indexer_setting', ?1)",
        (default_indexer_setting,),
    )?;

    conn.execute_batch(
        r#"
        -- Customize mapping, file type => file extensioin, e.g., {"plain_text":["txt", "my_own_txt"],"markdown":["md","mdx"]}
        insert or ignore into config (name, value) VALUES ('file_type_ext_mapping', '{"plain_text":[]}');
        insert or ignore into config (name, value) VALUES ('active_model_platform', 'openai');
        insert or ignore into config (name, value) VALUES ('fs_watcher_setting', '{"directories":[],"files":[]}');
        "#,
    )?;

    // Model platform init https://api.siliconflow.com/v1 or https://api.siliconflow.cn/v1
    conn.execute_batch(
        r#"
        insert or ignore into model_platform (name, title, office_site_url, base_url, is_proxy_enable) values ('openai', 'OpenAI', 'https://openai.com/', 'https://api.openai.com/v1', false);
        insert or ignore into model_platform (name, title, office_site_url, base_url, is_proxy_enable) values ('deepseek', 'DeepSeek', 'https://www.deepseek.com/', 'https://api.deepseek.com/v1', false);
        insert or ignore into model_platform (name, title, office_site_url, base_url, is_proxy_enable) values ('dashscope', 'DashScope', 'https://www.aliyun.com/product/bailian', 'https://dashscope.aliyuncs.com/api/v1', false);
        insert or ignore into model_platform (name, title, office_site_url, base_url, is_proxy_enable) values ('siliconflow', 'SiliconFlow-硅基流动', 'https://www.siliconflow.cn/', 'https://api.siliconflow.cn/v1', false);
        "#,
    )?;

    // Model init
    conn.execute_batch(r#"
    begin transaction;

    -- Text model
    insert or ignore into ai_model (name, title, model_types, platform, context_window, max_input_tokens, max_output_tokens, remark, is_enable)
    values ('deepseek-chat', 'deepseek-v3', 'text', 'deepseek', 131072, 126976, 8192, 'Non-thinking Model', true);

    insert or ignore into ai_model (name, title, model_types, platform, context_window, max_input_tokens, max_output_tokens, is_reasoner, is_thinking_closable, remark, is_enable)
    values ('deepseek-reasoner', 'deepseek-r1', 'text', 'deepseek', 131072, 65536, 65536, true, false, 'Thinking Model', true);

    insert or ignore into ai_model (name, title, model_types, platform, input_types, context_window, max_input_tokens, max_output_tokens, remark, properties, is_enable)
    values ('gpt-5-mini', 'gpt-5-mini', 'text,vision', 'openai', 'text,image', 400000, 272000, 128000, 'GPT-5 mini is a faster, more cost-efficient version of GPT-5. It''s great for well-defined tasks and precise prompts.', '{"supported_file_exts:["png","jpeg","jpg","webp","gif"],"total_size_limits":52428800,image_limits:500"}', true);

    insert or ignore into ai_model (name, title, model_types, platform, context_window, max_input_tokens, max_output_tokens, is_reasoner, is_thinking_closable, is_enable)
    values ('qwen-turbo', '通义千问turbo', 'text', 'dashscope', 131072, 98304, 16384, true, true, true);

    insert or ignore into ai_model (name, title, model_types, platform, remark, is_free, is_enable)
    values ('thudm/glm-z1-9b-0414', 'glm-z1-9b', 'text', 'siliconflow', 'GLM-Z1-9B-0414 是 GLM 系列的小型模型，仅有 90 亿参数，但保持了开源传统的同时展现出惊人的能力。| GLM-Z1-9B-0414 is a compact model in the GLM series, featuring only 9 billion parameters while maintaining the open-source tradition and demonstrating remarkable capabilities.', true, true);
    
    insert or ignore into ai_model (name, title, model_types, platform, input_types, is_enable)
    values ('qwen2-vl-7b-instruct', 'vl-7b-instruct', 'vision', 'dashscope', 'text,image', true);

    insert or ignore into ai_model (name, title, model_types, platform, input_types, remark, is_free, is_enable)
    values ('THUDM/GLM-4.1V-9B-Thinking', 'GLM-4.1V-9B-Thinking', 'vision', 'siliconflow', 'text,image', 'GLM-4.1V-9B-Thinking 是由智谱 AI 和清华大学 KEG 实验室联合发布的一款开源视觉语言模型（VLM），专为处理复杂的多模态认知任务而设计。该模型基于 GLM-4-9B-0414 基础模型，通过引入“思维链”（Chain-of-Thought）推理机制和采用强化学习策略，显著提升了其跨模态的推理能力和稳定性。作为一个 9B 参数规模的轻量级模型，它在部署效率和性能之间取得了平衡，在 28 项权威评测基准中，有 18 项的表现持平甚至超越了 72B 参数规模的 Qwen-2.5-VL-72B。该模型不仅在图文理解、数学科学推理、视频理解等任务上表现卓越，还支持高达 4K 分辨率的图像和任意宽高比输入 | GLM-4.1V-9B-Thinking is an open-source Vision Language Model (VLM) jointly released by Zhipu AI and Tsinghua University''s KEG Laboratory, specifically designed for handling complex multimodal cognitive tasks. Based on the GLM-4-9B-0414 foundation model, it significantly enhances its cross-modal reasoning capabilities and stability through the introduction of Chain-of-Thought reasoning mechanisms and reinforcement learning strategies. As a lightweight model with 9B parameters, it strikes a balance between deployment efficiency and performance. Among 28 authoritative evaluation benchmarks, it has achieved performance equal to or surpassing that of the 72B-parameter Qwen-2.5-VL-72B in 18 of them. The model not only excels in tasks such as image-text understanding, mathematical and scientific reasoning, and video comprehension, but also supports images up to 4K resolution and arbitrary aspect ratios.', true, true);

    -- Speech-to-text
    insert or ignore into ai_model (name, title, model_types, platform, input_types, context_window, max_input_tokens, max_output_tokens, remark, is_enable)
    values ('gpt-4o-mini-transcribe', 'GPT-4o mini Transcribe(Speech-to-text)', 'asr', 'openai', 'text,audio',16000, 14000, 2000, 'GPT-4o mini Transcribe is a speech-to-text model that uses GPT-4o mini to transcribe audio. It offers improvements to word error rate and better language recognition and accuracy compared to original Whisper models. Use it for more accurate transcripts.', true);

    insert or ignore into ai_model (name, title, model_types, platform, input_types, remark, is_free, is_enable)
    values ('funaudiollm/sensevoicesmall', 'sensevoicesmall', 'asr', 'siliconflow', 'text,audio', 'SenseVoice 是一个具有多种语音理解能力的语音基础模型，包括自动语音识别（ASR）、口语语言识别（LID）、语音情感识别（SER）和音频事件检测（AED）。它支持 50 多种语言的多语言语音识别，在中文和粤语识别方面表现优于 Whisper 模型。此外，它还具有出色的情感识别和音频事件检测能力。该模型处理 10 秒音频仅需 70 毫秒，比 Whisper-Large 快 15 倍 | SenseVoice is a foundational speech model with multiple speech understanding capabilities, including Automatic Speech Recognition (ASR), Spoken Language Identification (LID), Speech Emotion Recognition (SER), and Audio Event Detection (AED). It supports multilingual speech recognition in over 50 languages, outperforming Whisper models in Chinese and Cantonese recognition. Additionally, it features excellent emotion recognition and audio event detection capabilities. The model processes 10 seconds of audio in just 70 milliseconds, making it 15 times faster than Whisper-Large.', true, true);
    commit;
    "#)?;

    conn.execute(
        "insert or ignore into config (name, value) VALUES (?1, ?2)",
        ("db_version", "1"),
    )?;
    Ok(())
}
