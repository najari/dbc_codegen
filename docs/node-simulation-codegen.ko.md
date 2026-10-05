# CANLOG 연결을 위한 DBC 생성기 보완

2026-10-06에 `8d0af24f258d356afc8c3a31d7a1f7de04e7209a`를 기준으로 보완했다.
입력 요구조건은 CANLOG의 `dbc-codegen-node-simulation.ko.md` 및 영어판이다.
이 저장소에서 수정한 범위는 코드 생성기(CG-01~08)다. CANLOG의 노드 상태,
가상 시간 스케줄러, 버스, GUI 연결은 별도 구현 범위이며 여기서 완료로 판정하지 않는다.

## 변경 내용과 지원 범위

| 요구조건 | 구현 및 제한 |
|---|---|
| CG-01 | 메시지 타입, 신호 접근자·상수, enum, mux 보조 타입과 공통 이름을 등록한다. 충돌에 결정적인 접미사를 붙인다. enum label 충돌도 별도 variant로 보존한다. 원본 이름·키와 생성 이름을 manifest에 남긴다. 동일 CAN identity의 중복 정의는 모호한 dispatch를 막기 위해 오류로 반환한다. 사용자 지정 attribute 상수의 충돌은 기존처럼 명시적으로 거부한다. |
| CG-02 | `SIG_VALTYPE_` float32/64의 길이를 검사하고 Intel/Motorola에서 `from_bits`/`to_bits`를 사용한다. 물리 setter는 NaN·Infinity를 거부한다. raw setter는 모든 비트 패턴을 보존한다. unscaled IEEE getter/setter는 signed zero를 보존한다. |
| CG-03 | 하나의 unscaled unsigned selector와 `mN` 단순 mux를 지원한다. 1비트 selector도 정수 API로 처리한다. 선택된 신호 비트만 복사하므로 1→0 변경이 적용되고, 공통 신호·비활성 영역은 보존된다. `SG_MUL_VAL_`, 중첩·독립 다중 selector는 명시적으로 거부한다. 순환·누락 switch가 포함된 확장 정의도 지원으로 간주하지 않는다. |
| CG-04 | 정수 wire 신호의 `f64` 물리 API와 truncate / nearest-away / exact 정책을 제공한다. 정수 변환은 wire 범위를 검사하며, 정수 물리 계산에는 i128 중간값을 사용한다. 숫자 신호의 `set_*_quantized`는 실제 설정된 물리값을 반환한다. raw setter는 물리 min/max를 우회하지만 wire 폭을 검사한다. `[0\|0]`은 기본 설정에서 0 범위로 유지한다. |
| CG-05 | 원본 ID를 파서의 u16 축소·확장 비트 masking 전에 검사한다. CAN ID, FD 길이, 1~64비트 폭, bit 범위, 활성 신호 겹침, factor/offset, min/max, mux, value-description 키를 검사한다. 일부 오류에는 원본 행 번호를 포함한다. generation 오류가 기존 파일을 비우지 않는다. |
| CG-06 | UTF-8/BOM, 명시적인 Windows-1252 및 CP949를 지원한다. 실패 바이트를 replacement 문자로 바꾸거나 인코딩을 추정하지 않는다. 한국어 경로·unit·label을 시험한다. DBC 메시지/신호 식별자의 Unicode 문법은 upstream 파서의 ASCII 문법에 제한되며, 미지원 식별자는 parse 오류로 반환한다. |
| CG-07 | 선택 노드의 BO_ 송신, BO_TX_BU_ 추가 송신 및 신호 수신 관계로 메시지를 선택한다. 모든 원본 정의에 이름을 배정한 후 필터링하므로 노드 선택이 공통 메시지의 이름을 바꾸지 않는다. 미지정 노드는 오류다. manifest는 원본 순번·이름·표준/확장 ID·송신자·수신자와 신호 배치를 보존한다. 입력 전체의 핵심 정의·VAL_/SIG_VALTYPE_ 참조를 검사한 후 필터링한다. |
| CG-08 | 정확한 입력 바이트 SHA-256, 생성기 기준 커밋·소스 hash, 옵션, 이름/키 매핑과 코드 hash를 기록한다. 코드와 manifest를 준비한 후 게시하고 I/O 실패 시 기존 결과를 복원한다. 복원 실패 시 백업 경로를 오류에 남긴다. `cache_matches`가 형상과 파일 변조를 검사한다. 이 보수적인 검사는 메모리에서 재생성한다. |

`VAL_`의 signed 예약값은 음수 또는 같은 unsigned wire 패턴으로 선언할 수 있다.
예를 들어 signed 8비트의 `128 "SNA"`는 wire `80`이며 접근자에서 -128로
해석된다. manifest에는 원본 128과 canonical -128을 모두 남긴다. 같은 비트에
서로 다른 중복 키가 배정되면 거부한다. 예약값이 물리 범위 밖이면 정상 enum
setter도 거부한다. fault injection은 명시적인 raw setter로 수행한다.

## CLI 사용

출력 디렉터리를 먼저 만든다. 성공 시 `messages.rs`와 `manifest.json`을 게시한다.

```powershell
cargo build --locked -p dbc-codegen-cli
New-Item -ItemType Directory -Force .\generated
.\target\debug\dbc-codegen.exe .\dbc_samples\example.dbc .\generated
```

노드, 정밀도와 반올림을 선택할 수 있다.

```powershell
.\target\debug\dbc-codegen.exe .\dbc_samples\example.dbc .\generated `
  --node FanController --physical-f64 --rounding nearest-away
```

`--node`는 반복해서 지정한다. 비어 있으면 모든 실제 메시지를 생성한다.
`--encoding`의 값은 `utf8`(기본), `windows1252`, `cp949`다.
`--rounding`의 값은 `truncate`(기본), `nearest-away`, `exact`다.
nearest-away는 가까운 정수로 반올림하며 정확히 절반일 때 0에서 멀어지는
방향을 택한다. exact는 표현할 수 없는 값을 오류로 반환한다.

생성기의 옵션만으로 주기 송신이나 수신 반응을 실행하지 않는다.
`embedded_can::Frame::dlc()`는 byte 길이다. CANLOG의 wire DLC 코드와
`fd/brs/esi`는 연결 어댑터가 별도로 설정해야 한다.

## API 변경과 게시 계약

- raw setter가 `Result<(), CanError>`를 반환한다. 호출자는 오류를 처리해야 한다.
- enum 값은 정확한 raw 값이다. `_Other`도 해당 signed/unsigned raw 타입을 받는다.
  factor/offset이 있는 enum을 물리값으로 잘못 역변환하지 않는다.
- 물리 정수 범위 상수는 i128, 실수 범위 상수는 f64다. 정수 범위에 소수 경계가
  있으면 표현 가능한 정수에 맞춰 min을 올리고 max를 내린다.
- `physical_f64`는 일반 숫자 신호의 API를 바꾼다. bool, enum, mux selector는
  원래 역할의 타입을 유지한다. IEEE wire float32는 f32, float64는 f64다.
- f64는 모든 u64 값을 정확하게 표현하지 못한다. 정확한 64비트 값은 raw API로
  설정한다. 물리 setter는 변환 뒤 범위를 검사하여 saturating cast나 wrap을 허용하지 않는다.
- 파일 게시 두 번의 rename은 프로세스 강제 종료에 대해 하나의 원자적 작업이 아니다.
  사용 전에 코드 hash 또는 `cache_matches`로 코드/manifest 쌍을 확인한다.
  generation 실패에는 임시 결과를 게시하지 않는다.
- manifest의 revision은 기준 Git commit이다. 미커밋 수정은 generator source hash로
  구별한다. timestamp를 출력에 넣지 않아 같은 입력·소스·옵션에서 결과를 재현한다.

## 검증 절차와 기록

합성 fixture는 이 프로젝트에서 작성했다. `dbc_samples`와 외부 Comfort 원본은
수정하지 않았으며 샘플별 입력 hash와 시험 후 동일 여부를 보고서에 남겼다.

```powershell
cargo test --locked -p dbc-codegen --features std --lib `
  --test attribute_structs --test padding_bit_value --test simulation
cargo test --locked -p dbc-codegen-cli
cargo clippy --locked -p dbc-codegen -p dbc-codegen-cli --lib --bins -- -D warnings
cargo +1.88.0 test --locked -p dbc-codegen --features std --lib `
  --test attribute_structs --test padding_bit_value --test simulation --target-dir target/msrv
cargo run --locked --example verify_samples -- dbc_samples artifacts/sample-report.json
python scripts/verify_dll.py --dll C:\Users\admin\codex\asc_parser_engine\engines\dbc\candb.dll
```

실제 생성물을 rustc로 컴파일하고 실행한다. 1~64비트, signed/unsigned,
Intel/Motorola의 256조합과 독립적인 비트 배치 결과를 비교한다. IEEE ±0,
부호·유한값·NaN·Infinity raw 왕복, setter 실패 후 payload 보존, 1비트/일반 mux,
반올림·양자화, signed 예약값, 인코딩, 이름 충돌, manifest 재현·변조 검사 및
게시 복원을 함께 시험한다. 생성물의 `no_std` 소스 컴파일도 수행한다.
이는 임베디드 장치·다른 target에서 실행한 증거는 아니다.

로컬 보고서는 다음과 같다.

최종 확인 결과: Rust 1.98.1에서 library/속성/padding/생성물 시험 48개와 CLI
시험 1개가 통과했다. Rust 1.88.0에서 같은 library 시험 48개 및 CLI check가
통과했다. Clippy `-D warnings`, rustfmt 및 diff whitespace 검사도 통과했다.
제공된 샘플 88개 중 69개는 생성·컴파일에 성공하고 19개는 명시적으로 거부됐다.
생성이 허용된 코드의 컴파일 실패는 0개다. Model3CAN의 마지막 거부 사유는
`GTW_numberHVILNodes`의 wire 범위를 벗어난 `VAL_` 키다. 원본을 자동 수정하지 않았다.
Comfort 전체 1개와 DLL 비교 12개도 통과했다.

- `artifacts/sample-report.json`: 제공된 88개 샘플의 생성·컴파일 판정과 원본 hash.
- `artifacts/comfort-report.json`: 문서에 지정된 CANSystemDemo Comfort 전체의
  생성·컴파일과 `Diag_Request`/`DiagRequest` 매핑.
- `artifacts/dll-report.json`: 생성·컴파일·실행한 12개 payload, literal byte,
  candb.dll의 exact raw·물리값·활성 신호·상태 비교와 DLL hash.

DLL 비교에는 IEEE signed zero의 물리 부호 차이를 기록한다. DLL은 scaling에서
-0.0을 +0.0으로 바꿀 수 있다. wire 비트는 정확히 비교하며 생성기의 signed zero
보존은 별도 시험으로 확인한다. 일치한 DLL 결과만을 정확성의 단독 근거로 삼지 않는다.

적용 결과와 제외 이유는 보고서를 기준으로 확인한다. 확장 mux의 일반 지원,
CANLOG Frame/DLC 어댑터, 노드 시간·상태 시험(V-08/09), 기록·TUI·GUI 시험과
전체 upstream snapshot 재승인은 이 생성기 보완의 검증 완료 항목에 포함하지 않는다.
기존 snapshot은 원본 API를 대상으로 한 자료이므로 새 API의 컴파일 검증은
합성 fixture 및 제공된 실제 샘플에서 수행했다.
