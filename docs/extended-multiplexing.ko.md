# 확장 멀티플렉싱 설계 및 검증

## 목적

`dbc_samples`에서 거부된 확장 mux 샘플 13개의 구조를 지원한다. 기존 단순
unsigned mux의 타입과 API는 유지한다. 샘플 원본을 수정하지 않는다.

## 관계와 검증

- `SG_MUL_VAL_`는 대상 신호, 부모 selector, 닫힌 값 범위의 관계다.
  명시적 관계가 있으면 `mN` 대신 그 범위를 사용한다. 같은 부모에 대한
  여러 정의는 합집합으로 정규화한다. 서로 다른 부모를 지정한 한 신호는
  의미가 모호하므로 오류로 거부한다.
- `M`과 `mNM` 모두 selector 역할을 가진다. 명시적인 부모 관계가 있는
  `M`도 중첩 selector로 처리한다. 관계가 없는 `mN`/`mNM`은 유일한
  최상위 selector를 부모로 삼는다. 부모를 결정할 수 없으면 거부한다.
- 누락된 메시지/신호/selector, selector가 아닌 부모, 자기 참조, 순환,
  역전 범위와 wire 폭 초과를 거부한다. selector는 정수·factor=1·offset=0이다.
- 선택 조건은 부호 선언과 무관하게 width비트의 unsigned wire 패턴으로
  비교한다. signed getter는 부호 확장값을 반환한다. 음수 mux 키 문법은
  upstream 파서가 지원하지 않으므로 이번 범위에 포함하지 않는다.
- 신호의 활성 조건은 모든 조상 조건의 AND이며, 한 부모의 여러 범위는
  OR다. 같은 selector에 대한 범위가 교차하지 않으면 서로 배타적이다.
  동시에 활성화될 수 있는 신호의 실제 비트가 겹치면 거부한다.
- 범위 전체나 독립 selector의 모든 조합을 열거하지 않는다.

## 생성 API

확장 관계, 복수 selector 또는 signed selector를 가진 메시지는 아래 API를
사용한다. 기존 단순 unsigned mux는 기존 분기 타입/API를 계속 사용한다.

- `new() -> Result<Self, CanError>`: padding 정책에 따라 비어 있는 payload를
  만든다. selector 조건에 맞는 초기 데이터 설정은 호출자가 명시한다.
- `<signal>_is_active() -> bool`: 조상 조건을 포함한 활성 여부.
- 조건부 `<signal>() -> Result<T, CanError>`: 비활성 상태에는
  `InactiveSignal { message_id, signal }`을 반환한다. 무조건 활성인 일반
  신호의 getter는 기존 반환 타입을 유지한다.
- `set_<signal>`, `set_<signal>_raw_val`, `set_<signal>_quantized`:
  조건부 신호는 활성 여부를 먼저 검사한다. 오류 시 payload는 그대로다.
  일반 물리 setter는 기존 물리 범위/반올림 검사를 유지한다.
- `<signal>_raw_val()`: 활성 여부와 무관하게 저장된 wire 값을 읽는
  명시적인 진단 API다. signed 신호는 부호 확장한다.
- `select_<selector>(value: u64)`: 부모 활성 조건과 wire 폭을 검사한 뒤
  unsigned wire 패턴을 설정한다. `[0|0]` 물리 범위를 적용하지 않는다.
  자식이 없는 선택값도 허용하며 그때 자식 신호는 비활성이다.
- 선택 신호 변경은 그 비트만 변경한다. 데이터 쓰기는 그 신호의 비트만
  변경한다. 비활성 신호와 공유하는 비트의 변화는 정상적인 재사용이다.
  비활성 영역을 자동 초기화하거나 다른 selector를 자동 선택하지 않는다.

selector의 enum label은 manifest에 보존하되, 선택 API는 raw 정수를 사용한다.
확장 mux의 `Arbitrary`는 임의 wire 데이터와 유효한 분기 조건을 생성한다.
물리 범위와 IEEE finite 조건을 만족하는 값만 생성한다고 보장하지 않는다.

합성 fixture의 사용 예는 다음과 같다.

```rust
let mut frame = Ranged::new()?;
frame.select_switch(3)?; // [0|0] 물리 범위와 분리된 wire 선택
assert!(frame.a_is_active() && frame.c_is_active());
frame.set_a(42)?;
assert_eq!(frame.a()?, 42);
assert!(!frame.b_is_active());
assert!(frame.set_b(1).is_err()); // 실패 시 frame의 바이트는 그대로다
```

호출 함수는 `Result<_, CanError>`를 반환하는 문맥으로 작성한다. 원래 지원하던
단순 mux는 `new(selector, ...)`, 분기 enum과 `set_mN` API를 계속 사용한다.
`CanError`에 새 변형이 추가됐으므로 외부의 exhaustive match는 새 변형을
처리하거나 마지막에 wildcard 분기를 둬야 한다.

## 메타데이터와 호환성

manifest의 신호별 mux 정보에는 selector 역할, 부모 원본/생성 이름, 정규화한
범위를 추가한다. 메시지의 `mux_api`는 `guarded` 또는 `legacy`다. schema 1에
필드를 추가하는 방식이며 기존 필드의 의미를 바꾸지 않는다. 노드 필터 전
전체 입력을 검증하고 이름을 할당한다.
확장 관계의 대상/부모 이름도 함께 재매핑한다. 새 모듈을 소스 hash에 포함한다.
기존 `InvalidMultiplexor`는 단순 mux 호환성을 위해 유지하고, 새 조건부 API는
값을 u16으로 축소하는 오류 표현을 사용하지 않는다.

## 검증 기준

- 13개 샘플을 원본 그대로 생성·컴파일하고 기존 69개도 컴파일한다.
- 범위 경계·공백, 중첩 부모 비활성, 독립 selector 조합, 원본/dumped
  동일 의미, signed selector와 `[0|0]` 선택을 실제 실행으로 확인한다.
- bit 보존, 1→0 쓰기, 실패 시 비변경, IEEE와 enum 조건부 접근을 확인한다.
- 누락/순환/비-selector 부모, 범위 역전/초과, 조건부 비트 겹침을 거부한다.
- 범위 판단과 활성 신호/원시값은 독립 바이트 벡터 및 기준 DLL과 비교한다.
- no_std, Arbitrary, 노드 필터와 이름 충돌, manifest/cache를 확인한다.
- 회귀시험, Rust 1.88 및 현재 stable, Clippy와 fmt를 실행한다.

## 결과

2026-10-06에 구현 및 검증했다. 기준 Git 커밋은
`3cfa426af8fb154b443c406c648c619cf457b7a1`이며 최종 소스 hash와
시험 기록은 `artifacts/verification-summary.json`에 저장했다.

| 검증 | 결과 |
|---|---|
| 기존 및 확장 회귀시험 | Rust 1.98.1에서 라이브러리 51개 + CLI 1개 통과 |
| 최소 Rust 버전 | Rust 1.88.0에서 라이브러리 51개 및 CLI check 통과 |
| 정적 검사 | Clippy `-D warnings`, rustfmt, diff 검사 통과 |
| 원본 샘플 88개 | 82개 생성·컴파일, 6개 거부, 허용된 생성물 컴파일 실패 0 |
| 기존 mux 거부 샘플 13개 | 모두 생성·컴파일 및 DLL 비교 통과 |
| 확장 mux 런타임 | 222개 payload의 활성 신호/원시 비트 일치, 1,750개 쓰기 검사 통과 |
| 기존 DLL 비교 | IEEE/64비트/단순 mux 등 12개 벡터 통과 |
| 원본 및 이름 보존 | 88개 입력 hash 일치, 기존 69개 생성 타입·필드명 일치 |
| 외부 Comfort | 전체 생성·컴파일 통과 |

`scripts/verify_extended_mux.py`는 실제 생성된 Rust 코드를 컴파일·실행한다.
각 selector 범위의 경계와 인접 값, 부모 활성/비활성 경우를 시험하고,
DLL의 활성 신호 집합과 정확한 wire 값에 대조한다. 별도의 비트 배치 함수로
대상 외 비트 보존, 1→0 쓰기, 비활성 쓰기 거부를 검사한다. 합성 회귀시험은
u64 최대 selector 범위, signed/unsigned 1비트 중첩 selector, IEEE/enum,
no_std, Arbitrary, 이름 변경과 노드 필터도 실제 컴파일·실행한다.

남은 6개 거부는 신호 길이 초과 1개, CAN ID 표기/범위 2개, 동시 활성 비트
겹침 1개, 10바이트 길이 정책 1개, Model3의 2비트 신호에 있는 범위 밖
`VAL_` 키 1개다. 원본을 수정하거나 검사를 해제해서 통과시키지 않았다.

재현 명령은 `docs/node-simulation-codegen.ko.md`의 검증 절을 참고한다.
DLL 비교 결과는 `artifacts/extended-mux-report.json`에 기록했다.
