use base64;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rand::{Rng, thread_rng};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct EncodeData {
    pub ua: String,
    pub href: String,
    pub device_pixel_ratio: f64,
    pub scroll_x: i32,
    pub scroll_y: i32,
    pub inner_width: i32,
    pub inner_height: i32,
    pub outer_width: i32,
    pub outer_height: i32,
    pub screen_x: i32,
    pub screen_y: i32,
    pub screen_width: i32,
    pub screen_height: i32,
    pub avail_width: i32,
    pub avail_height: i32,
    pub history_length: i32,
}

impl EncodeData {
    pub fn new(ua: String, href: String) -> Self {
        let mut rng = thread_rng();
        let ratios: [f64; 4] = [1.0, 1.25, 1.5, 2.0];
        EncodeData {
            ua,
            href,
            device_pixel_ratio: ratios[rng.gen_range(0..4)],
            scroll_x: 0,
            scroll_y: 0,
            inner_width: rng.gen_range(1500..1700),
            inner_height: rng.gen_range(700..900),
            outer_width: rng.gen_range(1500..1700),
            outer_height: rng.gen_range(800..1000),
            screen_x: 0,
            screen_y: 0,
            screen_width: rng.gen_range(1500..1700),
            screen_height: rng.gen_range(800..1000),
            avail_width: rng.gen_range(1400..1600),
            avail_height: rng.gen_range(700..900),
            history_length: rng.gen_range(1..4),
        }
    }
    
    
    pub fn encode(&self, index: usize) -> i32 {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
            let v = self.device_pixel_ratio * 10.0;
            
        let arr: [i32; 16] = [
            self.scroll_x,                                        // 0
            self.scroll_y,                                        // 1
            self.inner_width,                                     // 2
            self.inner_height,                                    // 3
            self.outer_width,                                     // 4
            self.outer_height,                                    // 5
            self.screen_x,                                        // 6
            self.screen_y,                                        // 7
            self.screen_width,                                    // 8
            self.screen_height,                                   // 9
            self.avail_width,                                     // 10
            self.history_length,                                  // 11
            self.ua.len() as i32,                                 // 12
            self.href.len() as i32,                               // 13
            if v == 0.0 { 10 } else { v as i32 },                 // 14
            (now_ms % 256) as i32,                                // 15
        ];

        (arr[index % 16] + arr[(3 * index) % 16] + 17 * index as i32) & 255
    }
}

// ============================================================
// CTokenField: 内部字段结构，存储 encode 后的各字段值
// ============================================================
struct CTokenField {
    h:      i32, // encode(1)
    f:      i32, // 点击次数
    y:      i32, // encode(4)
    b:      i32, // 页面切换次数
    z:      i32, // encode(3)
    q:      i32, // encode(2)
    v:      i32, // openWindow 次数
    k:      i32, // encode(5)
    g:      i32, // 页面停留时间(秒)
    u:      i32, // 请求时间间隔(秒)
    w:      i32, // encode(6)
    j:      i32, // encode(7)
    x:      i32, // encode(8)
    dollar: i32, // encode(9)
    z_big:  i32, // encode(10)
    ee:     i32, // encode(11)
}

impl CTokenField {
    fn from_encode_data(ecdata: &EncodeData) -> Self {
        CTokenField {
            h:      ecdata.encode(1),
            f:      0,
            y:      ecdata.encode(4),
            b:      0,
            z:      ecdata.encode(3),
            q:      ecdata.encode(2),
            v:      0,
            k:      ecdata.encode(5),
            g:      0,
            u:      0,
            w:      ecdata.encode(6),
            j:      ecdata.encode(7),
            x:      ecdata.encode(8),
            dollar: ecdata.encode(9),
            z_big:  ecdata.encode(10),
            ee:     ecdata.encode(11),
        }
    }

    fn encode(&self) -> String {
        let mut buf = [0u8; 16];

        let mut field_map: HashMap<usize, (i32, usize)> = HashMap::new();
        field_map.insert(0,  (self.h,      1));
        field_map.insert(1,  (self.f,      1));
        field_map.insert(2,  (self.q,      1));
        field_map.insert(3,  (self.b,      1));
        field_map.insert(4,  (self.z,      1));
        field_map.insert(5,  (self.y,      1));
        field_map.insert(6,  (self.v,      1));
        field_map.insert(7,  (self.k,      1));
        field_map.insert(8,  (self.g,      2));
        field_map.insert(10, (self.u,      2));
        field_map.insert(12, (self.w,      1));
        field_map.insert(13, (self.j,      1));
        field_map.insert(14, (self.x,      1));
        field_map.insert(15, (self.dollar, 1));

        let mut i: usize = 0;
        while i < 16 {
            if let Some(&(data, length)) = field_map.get(&i) {
                if length == 1 {
                    let val = if data > 255 { 255 } else { data };
                    buf[i] = val as u8;
                    i += 1;
                } else {
                    let val = if data > 65535 { 65535 } else { data };
                    buf[i] = ((val >> 8) & 0xFF) as u8;
                    buf[i + 1] = (val & 0xFF) as u8;
                    i += 2;
                }
            } else {
                let fallback = if self.z_big & 4 != 0 { self.q } else { self.ee };
                let val = if fallback > 255 { 255 } else { fallback };
                buf[i] = val as u8;
                i += 1;
            }
        }

        let mut result = [0u8; 32];
        for i in 0..16 {
            result[i * 2] = buf[i];
            result[i * 2 + 1] = 0x00;
        }

        STANDARD.encode(&result)
    }
}


pub struct CTokenGenerator {
    field:       CTokenField,
    when_gen:    SystemTime,
    last_submit: SystemTime,
    base_timer:  i32,
}

impl CTokenGenerator {
    pub fn new(_ticket_collection_t: i64, _time_offset: i64, _stay_time: i32, ua : Option<String>) -> Self {
        let mut rng = thread_rng();
        let ua = ua.unwrap_or_else(|| "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36".to_string());
        let ecdata = EncodeData::new(
            ua,
            "https://show.bilibili.com".to_string(),
        );

        CTokenGenerator {
            field:       CTokenField::from_encode_data(&ecdata),
            when_gen:    SystemTime::now(),
            last_submit: SystemTime::now(),
            base_timer:  rng.gen_range(10..101),
        }
    }

    pub fn generate_ctoken(&mut self, is_create_v2: bool) -> String {
        let mut rng = thread_rng();
        let elapsed = SystemTime::now()
            .duration_since(self.when_gen)
            .unwrap_or_default()
            .as_secs() as i32;

        if is_create_v2 {
           self.field.f = rng.gen_range(0..3);   // touchend 点击 0~2
            self.field.b = rng.gen_range(0..2);   // visibilitychange 0~1
            self.field.v = rng.gen_range(10..51); // byte6 = beforeunload 10~50（create 阶段的关键差异）
            self.field.g = self.base_timer + elapsed; // timer = 基线 + 已过秒
            self.field.u = elapsed;                   // timediff = 距开抢秒数
        } else {
            self.field.f = 0;                     // touchend = 0
            self.field.b = 0;                     // visibilitychange = 0
            self.field.v = rng.gen_range(1..4);   // byte6 = openWindow 1~3
            self.field.g = self.base_timer + elapsed; // timer
            self.field.u = 0;                     // timediff = 0
        }

        self.last_submit = SystemTime::now();
        log::debug!("生成 CToken，f(touch): {}, b(vis): {}, v(byte6): {}, g(timer): {}, u(diff): {}", self.field.f, self.field.b, self.field.v, self.field.g, self.field.u);
        let ctoken = self.field.encode();
        log::debug!("总ctoken: {}", ctoken);
        ctoken
    }
}