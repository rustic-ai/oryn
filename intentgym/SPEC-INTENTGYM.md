# IntentGym: Product & Design Specification

## Version 1.0

---

## 1. Executive Summary

### 1.1 What is IntentGym?

IntentGym is a benchmark harness for evaluating AI web agents using Oryn as the browser interface. It provides a scientific framework for measuring how different combinations of LLMs, agent architectures, and prompt strategies perform on standardized web automation tasks.

### 1.2 The Core Insight

**Oryn is the constant; everything else is a variable.**

Traditional web agent benchmarks conflate multiple factors: the browser interface, the LLM, the prompt engineering, and the agent architecture. When results vary, it's impossible to attribute causation. IntentGym isolates variables by holding the browser interface constant (Oryn) while systematically varying:

- Language models (GPT-4, Claude, Gemini, Llama, etc.)
- Agent architectures (ReAct, Reflexion, Plan-and-Act, etc.)
- Prompt strategies (minimal, verbose, few-shot, chain-of-thought)
- Agent frameworks (native, Google ADK, Swarm, etc.)

### 1.3 Why IntentGym Matters

**For Oryn Development**
- Quantifies the value proposition: "Oryn reduces observation tokens by X% while improving success rate by Y%"
- Enables regression testing as Oryn evolves
- Identifies which web patterns cause agent failures

**For Agent Researchers**
- Fair comparison across architectures using identical browser interface
- Reproducible experiments with standardized metrics
- Ablation studies isolating specific factors

**For Practitioners**
- Data-driven model selection for their use cases
- Prompt optimization with measurable outcomes
- Cost/performance tradeoff analysis

---

## 2. Design Principles

### 2.1 Oryn as the Constant

Every agent in IntentGym interacts with the web through Oryn's Intent Language. This ensures:

| Aspect | Guarantee |
|--------|-----------|
| **Observation Format** | All agents receive identical semantic observations |
| **Action Space** | All agents use identical Intent Language commands |
| **State Representation** | All agents see the same element IDs, patterns, and metadata |
| **Error Handling** | All agents receive identical error messages and hints |

This eliminates browser interface as a confounding variable.

### 2.2 Pluggable Everything

IntentGym is designed as a composition of interfaces:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         IntentGym Architecture                          │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   Pluggable                    Fixed                      Pluggable     │
│   ┌─────────┐                  ┌─────────┐               ┌─────────┐   │
│   │Benchmarks│                 │  Oryn   │               │  LLMs   │   │
│   ├─────────┤                  │Interface│               ├─────────┤   │
│   │WebArena │                  └────┬────┘               │ OpenAI  │   │
│   │Mind2Web │                       │                    │Anthropic│   │
│   │MiniWoB++│    ┌──────────────────┼──────────────────┐ │ Google  │   │
│   │WebShop  │    │                  │                  │ │ Ollama  │   │
│   │ Custom  │    │                  ▼                  │ │ Custom  │   │
│   └────┬────┘    │         ┌───────────────┐           │ └────┬────┘   │
│        │         │         │ Agent Runner  │           │      │        │
│        └────────►│         │               │◄──────────┘      │        │
│                  │         │ • Metrics     │                  │        │
│   ┌─────────┐    │         │ • Trajectories│    ┌─────────┐  │        │
│   │ Agents  │────┼────────►│ • Evaluation  │◄───│ Prompts │──┘        │
│   ├─────────┤    │         └───────────────┘    ├─────────┤           │
│   │ ReAct   │    │                              │ Minimal │           │
│   │Reflexion│    │         ┌───────────────┐    │ Verbose │           │
│   │Plan-Act │    │         │  Frameworks   │    │ ReAct   │           │
│   │ RALPH   │    └────────►│   Adapters    │    │Few-shot │           │
│   │ Custom  │              ├───────────────┤    │ Custom  │           │
│   └─────────┘              │ Google ADK    │    └─────────┘           │
│                            │ Swarm         │                          │
│                            │ Rustic AI     │                          │
│                            │ LangChain     │                          │
│                            └───────────────┘                          │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### 2.3 Metrics-First Design

Every interaction is instrumented. IntentGym captures:

| Category | Metrics |
|----------|---------|
| **Tokens** | Input/output per turn, observation tokens, history growth, cumulative total |
| **Cost** | Per-call cost, per-task cost, observation cost ratio |
| **Performance** | Success rate, partial completion, steps to completion |
| **Latency** | LLM inference time, Oryn execution time, total wall time |
| **Errors** | Error types, recovery attempts, failure modes |

### 2.4 Reproducibility

Every run is fully specified and reproducible:

```yaml
# Every result can be reproduced with this configuration
run_id: "webarena_gpt4_react_20250115_143022"
seed: 42
benchmark: "webarena"
task_subset: "all"  # or specific task IDs

llm:
  provider: "openai"
  model: "gpt-4-turbo"
  temperature: 0.0
  max_tokens: 4096

agent:
  type: "react"
  max_steps: 30
  
prompt:
  template: "verbose_cot"
  
oryn:
  mode: "headless"
  version: "1.0.0"
```

---

## 3. Core Abstractions

### 3.1 Oryn Interface

The browser interface is the only fixed component. All agents interact through this interface.

```python
class OrynInterface:
    """
    The constant in all experiments.
    Provides semantic browser control via Intent Language.
    """
    
    def observe(self, **options) -> OrynObservation:
        """
        Get structured observation of current page.
        
        Returns:
            OrynObservation containing:
            - raw: str          # Raw Intent Language response
            - url: str          # Current URL
            - title: str        # Page title
            - elements: list    # Interactive elements with IDs
            - patterns: list    # Detected UI patterns
            - available_intents: list  # Ready-to-use intents
            - token_count: int  # Tokens in this observation
        """
        pass
    
    def execute(self, command: str) -> OrynResult:
        """
        Execute an Intent Language command.
        
        Args:
            command: Intent Language command string
            
        Returns:
            OrynResult containing:
            - success: bool     # Whether command succeeded
            - raw: str          # Raw response
            - changes: list     # DOM changes detected
            - error: str?       # Error message if failed
        """
        pass
    
    # Convenience methods (delegate to execute)
    def goto(self, url: str) -> OrynResult: ...
    def click(self, target: str | int) -> OrynResult: ...
    def type(self, target: str | int, text: str) -> OrynResult: ...
    def select(self, target: str | int, value: str) -> OrynResult: ...
    def scroll(self, direction: str = "down") -> OrynResult: ...
    def wait(self, condition: str, timeout: int = 30) -> OrynResult: ...
```

### 3.2 LLM Provider Interface

Language models are pluggable. All providers implement a common interface.

```python
class LLMProvider(ABC):
    """
    Abstract LLM provider.
    Implementations: OpenAI, Anthropic, Google, Ollama, etc.
    """
    
    @abstractmethod
    def complete(self, messages: list[dict]) -> LLMResponse:
        """
        Generate completion from message history.
        
        Args:
            messages: List of {role, content} dicts
            
        Returns:
            LLMResponse containing:
            - content: str          # Generated text
            - input_tokens: int     # Prompt tokens
            - output_tokens: int    # Completion tokens
            - latency_ms: float     # API call duration
            - cost_usd: float       # Estimated cost
        """
        pass
    
    @abstractmethod
    def count_tokens(self, text: str) -> int:
        """Count tokens in text using model's tokenizer."""
        pass
    
    @property
    @abstractmethod
    def context_limit(self) -> int:
        """Maximum context window size."""
        pass
```

### 3.3 Agent Interface

Agent architectures implement a common decision loop.

```python
class Agent(ABC):
    """
    Abstract agent architecture.
    Implementations: ReAct, Reflexion, Plan-and-Act, RALPH, etc.
    """
    
    def __init__(
        self,
        llm: LLMProvider,
        prompt: PromptTemplate,
        oryn: OrynInterface,
        **config
    ):
        self.llm = llm
        self.prompt = prompt
        self.oryn = oryn
        self.config = config
    
    @abstractmethod
    def decide(
        self, 
        state: AgentState, 
        observation: OrynObservation
    ) -> AgentAction:
        """
        Given current state and observation, decide next action.
        
        Args:
            state: Current agent state (task, history, metrics)
            observation: Current page observation from Oryn
            
        Returns:
            AgentAction containing:
            - command: str      # Intent Language command
            - reasoning: str?   # Agent's reasoning (if available)
        """
        pass
    
    @abstractmethod
    def update(
        self, 
        state: AgentState, 
        action: AgentAction, 
        result: OrynResult
    ):
        """Update agent state after action execution."""
        pass
    
    def reset(self):
        """Reset agent for new task."""
        pass
```

### 3.4 Prompt Template Interface

Prompts are configurable and versioned.

```python
@dataclass
class PromptTemplate:
    """
    Configurable prompt template.
    Separates system instructions from observation formatting.
    """
    
    name: str
    version: str
    
    # Core prompts
    system: str                     # System message
    observation_format: str         # How to format observations
    action_format: str              # Expected action format
    
    # Optional enhancements
    few_shot_examples: list[dict] = None
    error_recovery_hints: str = None
    
    def format_observation(
        self, 
        observation: OrynObservation,
        task: str,
        history: list[dict]
    ) -> str:
        """Format observation for LLM consumption."""
        return Template(self.observation_format).substitute(
            observation=observation.raw,
            task=task,
            history=self._format_history(history),
            url=observation.url,
            title=observation.title,
        )
```

### 3.5 Benchmark Interface

Benchmarks provide tasks and evaluation criteria.

```python
class Benchmark(ABC):
    """
    Abstract benchmark.
    Implementations: WebArena, Mind2Web, MiniWoB++, WebShop, etc.
    """
    
    @property
    @abstractmethod
    def name(self) -> str:
        """Benchmark name."""
        pass
    
    @abstractmethod
    def load_tasks(self, subset: str = "all") -> list[Task]:
        """
        Load benchmark tasks.
        
        Args:
            subset: Task subset ("all", "train", "test", or specific IDs)
            
        Returns:
            List of Task objects
        """
        pass
    
    @abstractmethod
    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        """
        Evaluate task completion.
        
        Returns:
            Evaluation containing:
            - success: bool         # Binary success
            - partial_score: float  # 0.0 to 1.0 partial completion
            - criteria_met: dict    # Which criteria passed/failed
        """
        pass


@dataclass
class Task:
    """A single benchmark task."""
    
    task_id: str
    intent: str                     # Natural language task description
    start_url: str                  # Starting URL
    
    # Success criteria
    success_criteria: dict          # What defines success
    
    # Metadata
    difficulty: str = "medium"      # easy, medium, hard
    category: str = "general"       # Task category
    max_steps: int = 30             # Maximum allowed steps
    timeout_seconds: int = 300      # Maximum time
    
    # Optional
    annotations: dict = None        # Ground truth annotations
    hints: list[str] = None         # Optional hints for ablation
```

### 3.6 Framework Adapter Interface

For integration with external agent frameworks.

```python
class FrameworkAdapter(ABC):
    """
    Adapter to run Oryn through external agent frameworks.
    Implementations: Google ADK, Swarm, LangChain, etc.
    """
    
    @abstractmethod
    def create_oryn_tools(self, oryn: OrynInterface) -> list:
        """
        Create framework-specific tool definitions for Oryn.
        
        Returns:
            List of tool definitions in framework's format
        """
        pass
    
    @abstractmethod
    def create_agent(
        self,
        llm_config: dict,
        prompt: PromptTemplate,
        tools: list
    ):
        """
        Create agent using framework's conventions.
        
        Returns:
            Framework-specific agent object
        """
        pass
    
    @abstractmethod
    def run_task(
        self,
        agent,
        task: Task,
        oryn: OrynInterface,
        collector: MetricsCollector
    ) -> TaskResult:
        """
        Run task using framework's execution model.
        Collect metrics through provided collector.
        """
        pass
```

---

## 4. Metrics System

### 4.1 Metrics Hierarchy

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         Metrics Hierarchy                               │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │ Turn Metrics (per observe→decide→act cycle)                     │   │
│  ├─────────────────────────────────────────────────────────────────┤   │
│  │ • observation_tokens    - Tokens in Oryn observation            │   │
│  │ • history_tokens        - Tokens in conversation history        │   │
│  │ • system_tokens         - Tokens in system prompt               │   │
│  │ • total_input_tokens    - Total LLM input                       │   │
│  │ • output_tokens         - LLM output tokens                     │   │
│  │ • llm_latency_ms        - LLM API call time                     │   │
│  │ • oryn_latency_ms       - Oryn command execution time           │   │
│  │ • action_success        - Did the action succeed?               │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                    │                                    │
│                                    ▼ aggregate                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │ Task Metrics (per benchmark task)                               │   │
│  ├─────────────────────────────────────────────────────────────────┤   │
│  │ • success               - Binary task success                   │   │
│  │ • partial_score         - Partial completion (0.0-1.0)          │   │
│  │ • total_steps           - Steps taken                           │   │
│  │ • total_input_tokens    - Sum of all turn input tokens          │   │
│  │ • total_output_tokens   - Sum of all turn output tokens         │   │
│  │ • total_observation_tokens - Sum of all observation tokens      │   │
│  │ • observation_ratio     - observation_tokens / input_tokens     │   │
│  │ • total_cost_usd        - Estimated total cost                  │   │
│  │ • total_duration_ms     - Wall clock time                       │   │
│  │ • peak_context_tokens   - Maximum context window usage          │   │
│  │ • failed_actions        - Count of failed Oryn commands         │   │
│  │ • error_types           - Categorized errors encountered        │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                    │                                    │
│                                    ▼ aggregate                          │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │ Run Metrics (per benchmark run)                                 │   │
│  ├─────────────────────────────────────────────────────────────────┤   │
│  │ • success_rate          - Fraction of successful tasks          │   │
│  │ • mean_partial_score    - Average partial completion            │   │
│  │ • mean_steps            - Average steps per task                │   │
│  │ • mean_input_tokens     - Average input tokens per task         │   │
│  │ • mean_observation_tokens - Average observation tokens          │   │
│  │ • mean_observation_ratio - Average observation/input ratio      │   │
│  │ • mean_cost_usd         - Average cost per task                 │   │
│  │ • total_cost_usd        - Total cost for entire run             │   │
│  │ • mean_duration_ms      - Average task duration                 │   │
│  │ • context_utilization   - How much of context limit used        │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### 4.2 Token Breakdown

The key metric for validating Oryn's value proposition:

```
┌─────────────────────────────────────────────────────────────────────────┐
│                    Token Breakdown (per LLM call)                       │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  Total Input Tokens                                                     │
│  ═══════════════════════════════════════════════════════════════════   │
│  ████████████████████████████████████████████████████████████████████   │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ System Prompt (fixed)                                            │  │
│  │ ████████████                                                     │  │
│  │ ~500-2000 tokens depending on prompt template                    │  │
│  └──────────────────────────────────────────────────────────────────┘  │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ Task Description (fixed per task)                                │  │
│  │ ████                                                             │  │
│  │ ~50-200 tokens                                                   │  │
│  └──────────────────────────────────────────────────────────────────┘  │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ Oryn Observation (THIS IS WHAT WE'RE OPTIMIZING)                 │  │
│  │ ██████████████████████████                                       │  │
│  │ ~200-2000 tokens (Oryn) vs ~10,000-50,000 tokens (HTML/AXTree)   │  │
│  └──────────────────────────────────────────────────────────────────┘  │
│                                                                         │
│  ┌──────────────────────────────────────────────────────────────────┐  │
│  │ History (grows each step)                                        │  │
│  │ ████████████████████████████████████████████████████████████     │  │
│  │ ~100-500 tokens per step × number of steps                       │  │
│  └──────────────────────────────────────────────────────────────────┘  │
│                                                                         │
│  Key Metric: observation_ratio = observation_tokens / total_input       │
│                                                                         │
│  Oryn Target: < 20% of input tokens are observations                    │
│  HTML Baseline: > 80% of input tokens are observations                  │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### 4.3 Metrics Collection

```python
class MetricsCollector:
    """
    Collects metrics throughout a task run.
    Integrates with LLM providers and Oryn interface.
    """
    
    def __init__(
        self,
        task: Task,
        config: RunConfig,
        llm: LLMProvider
    ):
        self.task = task
        self.config = config
        self.llm = llm
        
        self.turns: list[TurnMetrics] = []
        self.start_time: float = None
        
    def start_task(self):
        """Begin timing the task."""
        self.start_time = time.time()
    
    def record_turn(
        self,
        observation: OrynObservation,
        llm_response: LLMResponse,
        action: AgentAction,
        result: OrynResult,
        token_breakdown: TokenBreakdown
    ):
        """Record metrics for a single turn."""
        turn = TurnMetrics(
            turn_number=len(self.turns) + 1,
            timestamp=time.time(),
            
            # Token metrics
            observation_tokens=observation.token_count,
            history_tokens=token_breakdown.history,
            system_tokens=token_breakdown.system,
            task_tokens=token_breakdown.task,
            total_input_tokens=llm_response.input_tokens,
            output_tokens=llm_response.output_tokens,
            
            # Latency metrics
            llm_latency_ms=llm_response.latency_ms,
            oryn_observe_latency_ms=observation.latency_ms,
            oryn_action_latency_ms=result.latency_ms if result else 0,
            
            # Cost
            cost_usd=llm_response.cost_usd,
            
            # Action outcome
            action_command=action.command,
            action_success=result.success if result else True,
            action_error=result.error if result and not result.success else None,
        )
        self.turns.append(turn)
    
    def finish_task(self, evaluation: Evaluation) -> TaskMetrics:
        """Finalize and return task metrics."""
        duration_ms = (time.time() - self.start_time) * 1000
        
        return TaskMetrics(
            task_id=self.task.task_id,
            config=self.config,
            
            # Outcome
            success=evaluation.success,
            partial_score=evaluation.partial_score,
            
            # Aggregated from turns
            total_steps=len(self.turns),
            total_input_tokens=sum(t.total_input_tokens for t in self.turns),
            total_output_tokens=sum(t.output_tokens for t in self.turns),
            total_observation_tokens=sum(t.observation_tokens for t in self.turns),
            total_cost_usd=sum(t.cost_usd for t in self.turns),
            total_duration_ms=duration_ms,
            
            # Derived
            observation_ratio=self._calc_observation_ratio(),
            peak_context_tokens=max(t.total_input_tokens for t in self.turns),
            failed_actions=sum(1 for t in self.turns if not t.action_success),
            
            # Raw turn data
            turns=self.turns,
        )
```

---

## 5. Benchmark Support

### 5.1 Supported Benchmarks

| Benchmark | Tasks | Focus | Source |
|-----------|-------|-------|--------|
| **WebArena** | 812 | Realistic web tasks across 5 domains | Stanford/CMU |
| **Mind2Web** | 2,350 | Cross-website generalization | OSU |
| **MiniWoB++** | 125 | Simplified web interaction primitives | OpenAI/Farama |
| **WebShop** | 12,000 | E-commerce product search | Princeton |
| **Custom** | Variable | User-defined tasks | User |

### 5.2 WebArena Loader

```python
class WebArenaLoader(Benchmark):
    """
    WebArena: A Realistic Web Environment for Building Autonomous Agents
    https://webarena.dev/
    
    812 tasks across 5 realistic web applications:
    - Shopping (OneStopShop)
    - Social Forum (Reddit clone)
    - CMS (GitLab)
    - Maps (OpenStreetMap)
    - Wikipedia
    """
    
    @property
    def name(self) -> str:
        return "webarena"
    
    def __init__(self, data_dir: Path = None, server_url: str = None):
        self.data_dir = data_dir or Path("~/.intentgym/webarena").expanduser()
        self.server_url = server_url  # WebArena server URL
    
    def load_tasks(self, subset: str = "all") -> list[Task]:
        tasks = []
        
        for task_file in self.data_dir.glob("*.json"):
            task_data = json.load(open(task_file))
            
            tasks.append(Task(
                task_id=task_data["task_id"],
                intent=task_data["intent"],
                start_url=self._resolve_url(task_data["start_url"]),
                success_criteria=self._parse_criteria(task_data["eval"]),
                difficulty=task_data.get("difficulty", "medium"),
                category=task_data.get("sites", ["general"])[0],
                annotations=task_data.get("reference_answers"),
            ))
        
        return self._filter_subset(tasks, subset)
    
    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        criteria = task.success_criteria
        results = {}
        
        # URL-based evaluation
        if "url_contains" in criteria:
            current_url = oryn.execute("url").raw
            results["url"] = criteria["url_contains"] in current_url
        
        # Element existence
        if "element_exists" in criteria:
            for selector in criteria["element_exists"]:
                exists = oryn.execute(f'exists "{selector}"').raw
                results[f"exists_{selector}"] = "true" in exists.lower()
        
        # Text content
        if "text_contains" in criteria:
            page_text = oryn.execute("text").raw
            for text in criteria["text_contains"]:
                results[f"text_{text}"] = text in page_text
        
        # Page state evaluation using Oryn's pattern detection
        if "pattern_present" in criteria:
            observation = oryn.observe()
            for pattern in criteria["pattern_present"]:
                results[f"pattern_{pattern}"] = pattern in observation.patterns
        
        all_passed = all(results.values())
        partial = sum(results.values()) / len(results) if results else 0
        
        return Evaluation(
            success=all_passed,
            partial_score=partial,
            criteria_met=results,
        )
```

### 5.3 Mind2Web Loader

```python
class Mind2WebLoader(Benchmark):
    """
    Mind2Web: Towards a Generalist Agent for the Web
    https://osu-nlp-group.github.io/Mind2Web/
    
    2,350 tasks across 137 real-world websites.
    Tests cross-website generalization.
    """
    
    @property
    def name(self) -> str:
        return "mind2web"
    
    def load_tasks(self, subset: str = "all") -> list[Task]:
        # Load from Mind2Web dataset format
        # Tasks include ground-truth action sequences
        pass
    
    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        # Mind2Web uses action-level evaluation
        # Compare predicted actions to ground truth
        pass
```

### 5.4 MiniWoB++ Loader

```python
class MiniWoBLoader(Benchmark):
    """
    MiniWoB++: Reinforcement Learning on Web Interfaces
    https://miniwob.farama.org/
    
    125 simplified web interaction tasks.
    Good for testing basic capabilities.
    """
    
    @property
    def name(self) -> str:
        return "miniwob"
    
    def __init__(self, server_url: str = "http://localhost:8765"):
        self.server_url = server_url
        self.env = None
    
    def load_tasks(self, subset: str = "all") -> list[Task]:
        # Each MiniWoB task is a separate "environment"
        task_names = [
            "click-button", "click-link", "click-option",
            "enter-text", "focus-text", "choose-date",
            "login-user", "search-engine", "email-inbox",
            # ... 125 total
        ]
        
        return [
            Task(
                task_id=name,
                intent=self._get_intent(name),  # Task-specific instruction
                start_url=f"{self.server_url}/miniwob/{name}",
                success_criteria={"env_success": True},  # MiniWoB returns success
                max_steps=10,
            )
            for name in task_names
        ]
    
    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        # MiniWoB environments have built-in success detection
        # Query the environment's success state
        result = oryn.execute("execute window.__miniwob_success__")
        success = "true" in result.raw.lower()
        
        return Evaluation(
            success=success,
            partial_score=1.0 if success else 0.0,
            criteria_met={"env_success": success},
        )
```

### 5.5 WebShop Loader

```python
class WebShopLoader(Benchmark):
    """
    WebShop: Towards Scalable Real-World Web Interaction with Grounded Language Agents
    https://webshop-pnlp.github.io/
    
    12,000 e-commerce tasks with human instructions.
    Tests product search and purchase flow.
    """
    
    @property
    def name(self) -> str:
        return "webshop"
    
    def load_tasks(self, subset: str = "all") -> list[Task]:
        # WebShop uses human-written product search instructions
        # e.g., "I need a red dress under $50 with good reviews"
        pass
    
    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        # WebShop uses attribute matching score
        # Compare selected product attributes to instruction requirements
        pass
```

### 5.6 Custom Benchmark

```python
class CustomBenchmark(Benchmark):
    """
    User-defined benchmark tasks.
    Load from YAML/JSON configuration.
    """
    
    def __init__(self, config_path: Path):
        self.config_path = config_path
        self.config = yaml.safe_load(open(config_path))
    
    @property
    def name(self) -> str:
        return self.config.get("name", "custom")
    
    def load_tasks(self, subset: str = "all") -> list[Task]:
        return [
            Task(
                task_id=t["id"],
                intent=t["intent"],
                start_url=t["start_url"],
                success_criteria=t["success_criteria"],
                **t.get("options", {})
            )
            for t in self.config["tasks"]
        ]


# Example custom benchmark YAML:
# 
# name: my_company_tasks
# tasks:
#   - id: login_test
#     intent: "Log into the admin dashboard with test credentials"
#     start_url: "https://admin.mycompany.com/login"
#     success_criteria:
#       url_contains: "/dashboard"
#       text_contains: "Welcome"
#     options:
#       max_steps: 10
#       credentials:
#         username: "test@mycompany.com"
#         password: "${TEST_PASSWORD}"  # Environment variable
```

---

## 6. Agent Architectures

### 6.1 ReAct Agent

```python
class ReActAgent(Agent):
    """
    ReAct: Synergizing Reasoning and Acting in Language Models
    https://arxiv.org/abs/2210.03629
    
    Interleaves reasoning (Thought) with acting (Action).
    Each step: Thought → Action → Observation
    """
    
    def decide(
        self, 
        state: AgentState, 
        observation: OrynObservation
    ) -> AgentAction:
        # Build messages with ReAct format
        messages = [
            {"role": "system", "content": self.prompt.system},
        ]
        
        # Add history
        for step in state.history:
            messages.append({
                "role": "assistant",
                "content": f"Thought: {step['reasoning']}\nAction: {step['action']}"
            })
            messages.append({
                "role": "user",
                "content": f"Observation: {step['observation']}"
            })
        
        # Current turn
        messages.append({
            "role": "user",
            "content": self.prompt.format_observation(
                observation=observation,
                task=state.task,
                history=state.history
            )
        })
        
        # Get LLM response
        response = self.llm.complete(messages)
        
        # Update state metrics
        state.total_input_tokens += response.input_tokens
        state.total_output_tokens += response.output_tokens
        state.total_cost_usd += response.cost_usd
        
        # Parse Thought/Action from response
        thought, action = self._parse_react(response.content)
        
        return AgentAction(command=action, reasoning=thought)
    
    def _parse_react(self, response: str) -> tuple[str, str]:
        """Extract Thought and Action from ReAct format response."""
        thought = ""
        action = ""
        
        if "Thought:" in response:
            thought_start = response.index("Thought:") + 8
            thought_end = response.index("Action:") if "Action:" in response else len(response)
            thought = response[thought_start:thought_end].strip()
        
        if "Action:" in response:
            action_start = response.index("Action:") + 7
            action = response[action_start:].strip().split("\n")[0]
        
        return thought, action
```

### 6.2 Plan-and-Act Agent

```python
class PlanActAgent(Agent):
    """
    Plan-and-Act: Generate complete plan first, then execute.
    
    Phase 1: Given task and initial observation, generate step-by-step plan
    Phase 2: Execute plan steps sequentially, re-plan if obstacles
    """
    
    def decide(
        self, 
        state: AgentState, 
        observation: OrynObservation
    ) -> AgentAction:
        # Check if we need to (re)plan
        if not hasattr(state, 'plan') or state.plan is None:
            state.plan = self._generate_plan(state, observation)
            state.plan_index = 0
        
        # Check if plan is complete
        if state.plan_index >= len(state.plan):
            # Verify success or re-plan
            return AgentAction(
                command="observe",
                reasoning="Plan complete, verifying result"
            )
        
        # Execute next step
        action = state.plan[state.plan_index]
        state.plan_index += 1
        
        return AgentAction(
            command=action,
            reasoning=f"Executing plan step {state.plan_index}/{len(state.plan)}"
        )
    
    def _generate_plan(
        self, 
        state: AgentState, 
        observation: OrynObservation
    ) -> list[str]:
        """Generate execution plan from task and observation."""
        plan_prompt = f"""Task: {state.task}

Current page state:
{observation.raw}

Generate a step-by-step plan to accomplish this task.
Use Oryn Intent Language commands.
Output one command per line, nothing else.

Plan:"""
        
        messages = [
            {"role": "system", "content": self.prompt.system},
            {"role": "user", "content": plan_prompt}
        ]
        
        response = self.llm.complete(messages)
        
        # Parse plan steps
        lines = response.content.strip().split("\n")
        plan = [
            line.strip()
            for line in lines
            if line.strip() and not line.startswith("#")
        ]
        
        return plan
    
    def update(
        self, 
        state: AgentState, 
        action: AgentAction, 
        result: OrynResult
    ):
        super().update(state, action, result)
        
        # If action failed, trigger re-planning
        if not result.success:
            state.plan = None  # Will regenerate on next decide()
```

### 6.3 Reflexion Agent

```python
class ReflexionAgent(Agent):
    """
    Reflexion: Language Agents with Verbal Reinforcement Learning
    https://arxiv.org/abs/2303.11366
    
    Maintains reflections on past failures to improve future attempts.
    After failure, generates verbal feedback for future reference.
    """
    
    def __init__(self, **kwargs):
        super().__init__(**kwargs)
        self.reflections: list[str] = []
        self.max_reflections: int = 3
    
    def decide(
        self, 
        state: AgentState, 
        observation: OrynObservation
    ) -> AgentAction:
        messages = [{"role": "system", "content": self.prompt.system}]
        
        # Include relevant reflections
        if self.reflections:
            reflection_text = "\n".join(
                f"• {r}" for r in self.reflections[-self.max_reflections:]
            )
            messages.append({
                "role": "user",
                "content": f"Learn from past attempts:\n{reflection_text}"
            })
        
        # Standard ReAct-style history and observation
        # ...
        
        response = self.llm.complete(messages)
        return self._parse_action(response.content)
    
    def reflect_on_failure(self, state: AgentState, error: str):
        """
        Generate reflection after task failure.
        Called by runner when task fails.
        """
        reflection_prompt = f"""You failed to complete this task:

Task: {state.task}
Error: {error}

Actions taken:
{self._format_history(state.history)}

Reflect: What went wrong? What should you do differently next time?
Be specific and actionable. One paragraph."""
        
        response = self.llm.complete([
            {"role": "system", "content": "You are reflecting on a failed web automation task."},
            {"role": "user", "content": reflection_prompt}
        ])
        
        self.reflections.append(response.content.strip())
```

### 6.4 RALPH Agent

```python
class RALPHAgent(Agent):
    """
    RALPH: Retrieval Augmented Language Model for Planning in Hypertext
    
    Uses retrieval over past successful trajectories to guide current task.
    Requires a trajectory store built from successful runs.
    """
    
    def __init__(self, trajectory_store: "TrajectoryStore", **kwargs):
        super().__init__(**kwargs)
        self.trajectory_store = trajectory_store
    
    def decide(
        self, 
        state: AgentState, 
        observation: OrynObservation
    ) -> AgentAction:
        # Retrieve similar past trajectories
        similar_trajectories = self.trajectory_store.retrieve(
            task=state.task,
            observation=observation.raw,
            k=3
        )
        
        # Build prompt with retrieved examples
        messages = [{"role": "system", "content": self.prompt.system}]
        
        if similar_trajectories:
            examples = "\n\n".join(
                self._format_trajectory(t) for t in similar_trajectories
            )
            messages.append({
                "role": "user",
                "content": f"Similar successful tasks for reference:\n{examples}"
            })
        
        # Current task
        messages.append({
            "role": "user",
            "content": self.prompt.format_observation(
                observation=observation,
                task=state.task,
                history=state.history
            )
        })
        
        response = self.llm.complete(messages)
        return self._parse_action(response.content)


class TrajectoryStore:
    """
    Store and retrieve successful trajectories for RALPH.
    Uses embedding similarity for retrieval.
    """
    
    def __init__(self, embedding_model: str = "text-embedding-3-small"):
        self.trajectories: list[Trajectory] = []
        self.embeddings: list[np.ndarray] = []
        self.embedding_model = embedding_model
    
    def add(self, trajectory: Trajectory):
        """Add successful trajectory to store."""
        embedding = self._embed(trajectory.task + " " + trajectory.final_state)
        self.trajectories.append(trajectory)
        self.embeddings.append(embedding)
    
    def retrieve(
        self, 
        task: str, 
        observation: str, 
        k: int = 3
    ) -> list[Trajectory]:
        """Retrieve k most similar trajectories."""
        query_embedding = self._embed(task + " " + observation)
        
        # Cosine similarity
        similarities = [
            np.dot(query_embedding, emb) / (np.linalg.norm(query_embedding) * np.linalg.norm(emb))
            for emb in self.embeddings
        ]
        
        # Top-k
        top_indices = np.argsort(similarities)[-k:][::-1]
        return [self.trajectories[i] for i in top_indices]
```

---

## 7. Prompt Templates

### 7.1 Built-in Templates

```python
# Minimal - tests raw capability
MINIMAL = PromptTemplate(
    name="minimal",
    version="1.0",
    system="""You control a web browser via Oryn Intent Language.
Commands: goto, observe, click, type, select, scroll, wait, back
Respond with only the next command.""",
    observation_format="${observation}",
    action_format="Command: ",
)

# Verbose with chain-of-thought
VERBOSE_COT = PromptTemplate(
    name="verbose_cot",
    version="1.0",
    system="""You are a web automation agent controlling a browser via Oryn Intent Language.

## Available Commands
- goto <url>: Navigate to URL
- observe: Scan page for interactive elements (returns numbered list)
- click <id|"text">: Click element by ID number or text
- type <id|"text"> "value": Type into input field
- select <id> "option": Select dropdown option
- scroll [direction]: Scroll page
- wait <condition>: Wait for condition
- back/forward: Navigate history
- login <user> <pass>: High-level login intent

## Element Notation
[id] type/role "text" {modifiers}
- id: Use this number in commands (e.g., click 5)
- type: input, button, link, select, checkbox, etc.
- role: email, password, submit, search, etc.
- modifiers: required, disabled, checked, primary, etc.

## Patterns
Oryn detects common patterns like login_form, search_form, cookie_banner.
These indicate available high-level intents.

## Response Format
THOUGHT: Analyze current state, plan next action
COMMAND: <intent language command>

Always start with observe if you don't know the page state.""",
    observation_format="""## Current Page
${observation}

## Task
${task}

## Previous Actions
${history}""",
    action_format="THOUGHT: ",
)

# ReAct format
REACT = PromptTemplate(
    name="react",
    version="1.0",
    system="""You are a web agent using the ReAct framework.
For each step: Thought → Action → Observation cycle.

Actions use Oryn Intent Language:
- goto <url>: Navigate
- observe: Scan page elements
- click <id|"text">: Click
- type <id|"text"> "value": Type text
- select, scroll, wait, back, forward

Element IDs appear as [N] in observations. Use these numbers directly.

Format your response as:
Thought: <reasoning about current state and next step>
Action: <intent language command>""",
    observation_format="""Observation: ${observation}

Task: ${task}""",
    action_format="Thought: ",
)

# Few-shot with examples
FEW_SHOT = PromptTemplate(
    name="few_shot",
    version="1.0",
    system="""You control a web browser. Learn from these examples:

Example 1 - Login:
Observation: @ login.example.com "Sign In"
[1] input/email "Email" {required}
[2] input/password "Password" {required}
[3] button/submit "Sign in" {primary}
Task: Log in with user@test.com and password123
Action: type 1 "user@test.com"
---
Observation: @ login.example.com "Sign In"
[1] input/email "Email" {value: "user@test.com"}
[2] input/password "Password" {required}
[3] button/submit "Sign in" {primary}
Action: type 2 "password123"
---
Observation: @ login.example.com "Sign In"
[1] input/email "Email" {value: "user@test.com"}
[2] input/password "Password" {value: "••••••••••"}
[3] button/submit "Sign in" {primary}
Action: click 3
---
Result: Success - navigated to dashboard

Example 2 - Search:
Observation: @ google.com "Google"
[1] input/search "Search" {focused}
[2] button "Google Search"
Task: Search for "Oryn browser automation"
Action: type 1 "Oryn browser automation"
---
Observation: @ google.com "Google"
[1] input/search {value: "Oryn browser automation"}
[2] button "Google Search"
Action: click 2
---
Result: Success - search results displayed

Now complete the task:""",
    observation_format="${observation}\nTask: ${task}",
    action_format="Action: ",
    few_shot_examples=[...],
)
```

### 7.2 Custom Template Example

```yaml
# prompts/my_custom_prompt.yaml
name: enterprise_agent
version: "1.0"

system: |
  You are an enterprise web automation agent for ${company_name}.
  
  IMPORTANT GUIDELINES:
  - Never submit forms without confirmation
  - Log all actions for audit purposes
  - Prefer reading over clicking when possible
  
  Commands available:
  ${oryn_command_reference}
  
  Your response must follow this format:
  ANALYSIS: What you see on the page
  REASONING: Why you're taking this action
  ACTION: The Oryn command
  CONFIDENCE: high/medium/low

observation_format: |
  ## Page State
  URL: ${url}
  Title: ${title}
  
  ## Interactive Elements
  ${observation}
  
  ## Task
  ${task}
  
  ## Action History
  ${history}

action_format: "ANALYSIS: "

# Template variables filled at runtime
variables:
  company_name: "Acme Corp"
  oryn_command_reference: "..."
```

---

## 8. Framework Adapters

### 8.1 Google ADK Adapter

```python
class GoogleADKAdapter(FrameworkAdapter):
    """
    Adapter for Google Agent Development Kit.
    https://google.github.io/adk-docs/
    """
    
    def create_oryn_tools(self, oryn: OrynInterface) -> list:
        from google.generativeai import tools
        
        def browser_observe() -> str:
            """Scan the current page for interactive elements."""
            return oryn.observe().raw
        
        def browser_goto(url: str) -> str:
            """Navigate to a URL."""
            return oryn.goto(url).raw
        
        def browser_click(target: str) -> str:
            """Click an element by ID number or text."""
            return oryn.click(target).raw
        
        def browser_type(target: str, text: str) -> str:
            """Type text into an input element."""
            return oryn.type(target, text).raw
        
        def browser_execute(command: str) -> str:
            """Execute any Oryn Intent Language command."""
            return oryn.execute(command).raw
        
        return [
            tools.Tool.from_function(browser_observe),
            tools.Tool.from_function(browser_goto),
            tools.Tool.from_function(browser_click),
            tools.Tool.from_function(browser_type),
            tools.Tool.from_function(browser_execute),
        ]
    
    def create_agent(
        self,
        llm_config: dict,
        prompt: PromptTemplate,
        tools: list
    ):
        import google.generativeai as genai
        
        model = genai.GenerativeModel(
            model_name=llm_config["model"],
            tools=tools,
            system_instruction=prompt.system,
        )
        
        return model
    
    def run_task(
        self,
        agent,
        task: Task,
        oryn: OrynInterface,
        collector: MetricsCollector
    ) -> TaskResult:
        collector.start_task()
        chat = agent.start_chat()
        
        # Navigate to start
        oryn.goto(task.start_url)
        observation = oryn.observe()
        
        prompt = f"Task: {task.intent}\n\nCurrent page:\n{observation.raw}"
        
        steps = 0
        while steps < task.max_steps:
            # Send message - ADK handles tool calls automatically
            response = chat.send_message(prompt)
            steps += 1
            
            # Collect metrics from response
            # (ADK response includes token usage)
            collector.record_turn(...)
            
            # Check success
            evaluation = self.benchmark.evaluate(task, oryn)
            if evaluation.success:
                return collector.finish_task(evaluation)
            
            # Next observation
            observation = oryn.observe()
            prompt = f"Observation:\n{observation.raw}"
        
        # Max steps reached
        evaluation = self.benchmark.evaluate(task, oryn)
        return collector.finish_task(evaluation)
```

### 8.2 OpenAI Swarm Adapter

```python
class SwarmAdapter(FrameworkAdapter):
    """
    Adapter for OpenAI Swarm.
    https://github.com/openai/swarm
    """
    
    def create_oryn_tools(self, oryn: OrynInterface) -> list:
        # Swarm uses plain Python functions
        def observe() -> str:
            """Scan the current page for interactive elements."""
            return oryn.observe().raw
        
        def goto(url: str) -> str:
            """Navigate to a URL."""
            return oryn.goto(url).raw
        
        def click(target: str) -> str:
            """Click an element by ID or text."""
            return oryn.click(target).raw
        
        def type_text(target: str, text: str) -> str:
            """Type text into an input."""
            return oryn.type(target, text).raw
        
        def execute(command: str) -> str:
            """Execute any Oryn command."""
            return oryn.execute(command).raw
        
        return [observe, goto, click, type_text, execute]
    
    def create_agent(
        self,
        llm_config: dict,
        prompt: PromptTemplate,
        tools: list
    ):
        from swarm import Agent
        
        return Agent(
            name="OrynBrowser",
            instructions=prompt.system,
            functions=tools,
            model=llm_config["model"],
        )
    
    def run_task(
        self,
        agent,
        task: Task,
        oryn: OrynInterface,
        collector: MetricsCollector
    ) -> TaskResult:
        from swarm import Swarm
        
        client = Swarm()
        collector.start_task()
        
        oryn.goto(task.start_url)
        observation = oryn.observe()
        
        messages = [{
            "role": "user",
            "content": f"Task: {task.intent}\n\nPage:\n{observation.raw}"
        }]
        
        steps = 0
        while steps < task.max_steps:
            response = client.run(agent=agent, messages=messages)
            steps += 1
            
            # Swarm returns the full message history
            messages = response.messages
            
            # Collect metrics
            collector.record_turn(...)
            
            # Check success
            evaluation = self.benchmark.evaluate(task, oryn)
            if evaluation.success:
                return collector.finish_task(evaluation)
            
            # Continue with observation
            observation = oryn.observe()
            messages.append({
                "role": "user",
                "content": f"Observation:\n{observation.raw}"
            })
        
        evaluation = self.benchmark.evaluate(task, oryn)
        return collector.finish_task(evaluation)
```

---

## 9. Runner & CLI

### 9.1 Benchmark Runner

```python
class BenchmarkRunner:
    """
    Main harness for running benchmarks.
    Orchestrates agent, benchmark, and metrics collection.
    """
    
    def __init__(self, config: RunConfig):
        self.config = config
        self.results: list[TaskResult] = []
        
        # Initialize components
        self.oryn = OrynInterface(
            mode=config.oryn_mode,
            **config.oryn_options
        )
        self.llm = create_llm(
            provider=config.llm_provider,
            model=config.llm_model,
            **config.llm_options
        )
        self.prompt = load_prompt(config.prompt_template)
        self.benchmark = load_benchmark(config.benchmark)
        self.agent = self._create_agent()
    
    def run(
        self, 
        subset: str = "all",
        progress_callback: callable = None
    ) -> BenchmarkReport:
        """Run benchmark and return report."""
        tasks = self.benchmark.load_tasks(subset)
        
        for i, task in enumerate(tasks):
            if progress_callback:
                progress_callback(i, len(tasks), task.task_id)
            
            result = self._run_task(task)
            self.results.append(result)
            
            # Save checkpoint
            self._save_checkpoint()
        
        return self._generate_report()
    
    def _run_task(self, task: Task) -> TaskResult:
        """Run a single task with full instrumentation."""
        collector = MetricsCollector(
            task=task,
            config=self.config,
            llm=self.llm
        )
        collector.start_task()
        
        # Reset agent state
        self.agent.reset()
        state = AgentState(task=task.intent)
        
        # Navigate to start
        self.oryn.goto(task.start_url)
        
        try:
            while state.step_count < task.max_steps:
                # Observe
                obs_start = time.time()
                observation = self.oryn.observe()
                obs_latency = (time.time() - obs_start) * 1000
                
                # Build token breakdown for this turn
                token_breakdown = TokenBreakdown(
                    system=self.llm.count_tokens(self.prompt.system),
                    task=self.llm.count_tokens(task.intent),
                    observation=observation.token_count,
                    history=self._count_history_tokens(state.history),
                )
                
                # Agent decides
                action = self.agent.decide(state, observation)
                
                # Execute
                act_start = time.time()
                result = self.oryn.execute(action.command)
                act_latency = (time.time() - act_start) * 1000
                
                # Record metrics
                collector.record_turn(
                    observation=observation,
                    llm_response=self.agent.last_llm_response,
                    action=action,
                    result=result,
                    token_breakdown=token_breakdown
                )
                
                # Update agent state
                self.agent.update(state, action, result)
                
                # Check success
                evaluation = self.benchmark.evaluate(task, self.oryn)
                if evaluation.success:
                    return collector.finish_task(evaluation)
                
        except Exception as e:
            return collector.finish_task(
                Evaluation(success=False, partial_score=0.0, error=str(e))
            )
        
        # Max steps reached
        evaluation = self.benchmark.evaluate(task, self.oryn)
        
        # If using Reflexion, generate reflection on failure
        if isinstance(self.agent, ReflexionAgent) and not evaluation.success:
            self.agent.reflect_on_failure(state, "Max steps reached")
        
        return collector.finish_task(evaluation)
```

### 9.2 CLI Interface

```python
# intentgym/cli.py

import click
from pathlib import Path
from rich.console import Console
from rich.progress import Progress

console = Console()

@click.group()
@click.version_option()
def cli():
    """IntentGym - Benchmark harness for Oryn web agents."""
    pass


@cli.command()
@click.option("--config", "-c", type=Path, required=True,
              help="Run configuration YAML file")
@click.option("--output", "-o", type=Path, default=Path("results/"),
              help="Output directory for results")
@click.option("--subset", "-s", default="all",
              help="Task subset to run (all, train, test, or task IDs)")
@click.option("--resume", is_flag=True,
              help="Resume from checkpoint if available")
def run(config: Path, output: Path, subset: str, resume: bool):
    """Run a benchmark with specified configuration."""
    
    # Load config
    run_config = RunConfig.from_yaml(config)
    
    # Check for checkpoint
    checkpoint_path = output / f"{run_config.run_id}_checkpoint.json"
    if resume and checkpoint_path.exists():
        console.print(f"[yellow]Resuming from checkpoint...[/yellow]")
        runner = BenchmarkRunner.from_checkpoint(checkpoint_path)
    else:
        runner = BenchmarkRunner(run_config)
    
    # Run with progress bar
    with Progress() as progress:
        task_progress = progress.add_task(
            "[green]Running benchmark...", 
            total=len(runner.benchmark.load_tasks(subset))
        )
        
        def on_progress(i, total, task_id):
            progress.update(task_progress, completed=i)
            console.print(f"  Task {i+1}/{total}: {task_id}")
        
        report = runner.run(subset=subset, progress_callback=on_progress)
    
    # Save results
    output.mkdir(parents=True, exist_ok=True)
    report_path = output / f"{run_config.run_id}.json"
    report.save(report_path)
    
    # Print summary
    report.print_summary(console)
    console.print(f"\n[green]Results saved to {report_path}[/green]")


@cli.command()
@click.option("--results", "-r", type=Path, multiple=True, required=True,
              help="Result files to compare")
@click.option("--output", "-o", type=Path,
              help="Output file for comparison (optional)")
@click.option("--format", "-f", type=click.Choice(["table", "csv", "json"]),
              default="table", help="Output format")
def compare(results: tuple[Path], output: Path, format: str):
    """Compare multiple benchmark runs."""
    
    reports = [BenchmarkReport.load(r) for r in results]
    comparison = ComparisonReport(reports)
    
    if format == "table":
        comparison.print_table(console)
    elif format == "csv":
        csv_content = comparison.to_csv()
        if output:
            output.write_text(csv_content)
        else:
            print(csv_content)
    elif format == "json":
        json_content = comparison.to_json()
        if output:
            output.write_text(json_content)
        else:
            print(json_content)


@cli.command()
@click.option("--benchmark", "-b", required=True,
              type=click.Choice(["webarena", "mind2web", "miniwob", "webshop"]),
              help="Benchmark to run")
@click.option("--llms", "-l", multiple=True, 
              default=["gpt-4-turbo", "claude-sonnet-4-20250514"],
              help="LLMs to test")
@click.option("--agents", "-a", multiple=True,
              default=["react"],
              help="Agent architectures to test")
@click.option("--prompts", "-p", multiple=True,
              default=["minimal", "verbose_cot"],
              help="Prompt templates to test")
@click.option("--output", "-o", type=Path, default=Path("results/matrix/"),
              help="Output directory")
@click.option("--parallel", "-j", type=int, default=1,
              help="Number of parallel runs")
def matrix(benchmark: str, llms: tuple, agents: tuple, prompts: tuple,
           output: Path, parallel: int):
    """Run experimental matrix across configurations."""
    
    # Generate all configurations
    configs = []
    for llm in llms:
        for agent in agents:
            for prompt in prompts:
                provider = infer_provider(llm)
                configs.append(RunConfig(
                    run_id=f"{benchmark}_{llm}_{agent}_{prompt}",
                    benchmark=benchmark,
                    llm_provider=provider,
                    llm_model=llm,
                    agent_type=agent,
                    prompt_template=prompt,
                    oryn_mode="headless",
                ))
    
    console.print(f"[bold]Running {len(configs)} configurations...[/bold]")
    
    # Run matrix
    results = []
    for config in configs:
        console.print(f"\n[cyan]Running: {config.run_id}[/cyan]")
        runner = BenchmarkRunner(config)
        report = runner.run()
        report.save(output / f"{config.run_id}.json")
        results.append(report)
    
    # Generate comparison
    comparison = ComparisonReport(results)
    comparison.print_table(console)
    comparison.save(output / "comparison.json")


@cli.command()
@click.option("--benchmark", "-b", required=True, help="Benchmark name")
@click.option("--data-dir", "-d", type=Path, help="Data directory")
def download(benchmark: str, data_dir: Path):
    """Download benchmark data."""
    
    downloader = BenchmarkDownloader(benchmark, data_dir)
    
    with Progress() as progress:
        task = progress.add_task(f"[green]Downloading {benchmark}...", total=100)
        
        def on_progress(pct):
            progress.update(task, completed=pct)
        
        downloader.download(on_progress)
    
    console.print(f"[green]Downloaded {benchmark} to {data_dir}[/green]")


@cli.command()
@click.argument("result_file", type=Path)
@click.option("--task-id", "-t", help="Specific task to inspect")
@click.option("--format", "-f", type=click.Choice(["summary", "trajectory", "metrics"]),
              default="summary")
def inspect(result_file: Path, task_id: str, format: str):
    """Inspect benchmark results in detail."""
    
    report = BenchmarkReport.load(result_file)
    
    if task_id:
        task_result = report.get_task(task_id)
        if format == "trajectory":
            print_trajectory(task_result, console)
        elif format == "metrics":
            print_task_metrics(task_result, console)
        else:
            print_task_summary(task_result, console)
    else:
        if format == "metrics":
            print_aggregate_metrics(report, console)
        else:
            report.print_summary(console)


if __name__ == "__main__":
    cli()
```

### 9.3 Configuration Files

```yaml
# configs/webarena_gpt4_react.yaml
run_id: webarena_gpt4_react_v1
seed: 42

benchmark: webarena
benchmark_options:
  server_url: http://localhost:7860
  data_dir: ~/.intentgym/webarena

llm_provider: openai
llm_model: gpt-4-turbo
llm_options:
  temperature: 0.0
  max_tokens: 4096

agent_type: react
agent_options:
  max_history: 10  # Truncate history to last N steps

prompt_template: verbose_cot

oryn_mode: headless
oryn_options: {}

max_steps: 30
timeout_seconds: 300
```

```yaml
# configs/ablation_token_budget.yaml
run_id: ablation_token_budget_8k
seed: 42

benchmark: webshop
benchmark_options:
  data_dir: ~/.intentgym/webshop

llm_provider: anthropic
llm_model: claude-sonnet-4-20250514

agent_type: react
prompt_template: minimal

oryn_mode: headless

# Ablation: Cap context at 8K tokens
max_context_tokens: 8000
truncation_strategy: truncate_history  # or: truncate_observation, error

max_steps: 50
```

---

## 10. Reporting & Visualization

### 10.1 Summary Report

```
═══════════════════════════════════════════════════════════════════════════════
                         IntentGym Benchmark Report
═══════════════════════════════════════════════════════════════════════════════

Run ID:         webarena_gpt4_react_v1
Timestamp:      2025-01-15 14:30:22 UTC
Duration:       4h 23m 17s

Configuration:
  Benchmark:    WebArena (812 tasks)
  LLM:          GPT-4 Turbo (OpenAI)
  Agent:        ReAct
  Prompt:       verbose_cot
  Oryn Mode:    headless

───────────────────────────────────────────────────────────────────────────────
                              Performance
───────────────────────────────────────────────────────────────────────────────
Success Rate:           23.4%  (190/812)
Partial Completion:     47.2%  (mean across failed tasks)
Mean Steps:             18.3   (successful: 14.2, failed: 21.1)

───────────────────────────────────────────────────────────────────────────────
                              Token Usage
───────────────────────────────────────────────────────────────────────────────
Mean Input Tokens:      12,847  (±4,231)
Mean Output Tokens:     423     (±187)
Mean Observation Tokens: 1,847  (±892)    ← KEY METRIC
Observation Ratio:      14.4%             ← KEY METRIC

Context Usage:
  Mean Peak:            24,891  (19.4% of 128K limit)
  Max Peak:             67,234  (52.5% of 128K limit)

───────────────────────────────────────────────────────────────────────────────
                                Cost
───────────────────────────────────────────────────────────────────────────────
Mean Cost/Task:         $0.89
Total Cost:             $722.68
Cost Breakdown:
  Input Tokens:         $0.71/task (79.8%)
  Output Tokens:        $0.18/task (20.2%)

───────────────────────────────────────────────────────────────────────────────
                              Latency
───────────────────────────────────────────────────────────────────────────────
Mean Task Duration:     47.2s
  LLM Time:             32.1s  (68.0%)
  Oryn Time:            12.4s  (26.3%)
  Other:                2.7s   (5.7%)

───────────────────────────────────────────────────────────────────────────────
                            Error Analysis
───────────────────────────────────────────────────────────────────────────────
Failed Actions:         2.1 per task (mean)
Error Distribution:
  ELEMENT_NOT_FOUND:    847 (41.2%)
  TIMEOUT:              412 (20.0%)
  NAVIGATION_ERROR:     234 (11.4%)
  Other:                564 (27.4%)

═══════════════════════════════════════════════════════════════════════════════
```

### 10.2 Comparison Table

```
┌────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│                                          Benchmark Comparison                                                      │
├─────────────────────────────┬──────────┬────────┬────────────┬────────────┬───────────┬──────────┬─────────────────┤
│ Configuration               │ Success  │ Steps  │ Input Tok  │ Obs Tok    │ Obs Ratio │ Cost     │ Latency         │
├─────────────────────────────┼──────────┼────────┼────────────┼────────────┼───────────┼──────────┼─────────────────┤
│                             │          │        │            │            │           │          │                 │
│ Baseline Comparisons        │          │        │            │            │           │          │                 │
│ ─────────────────────────── │          │        │            │            │           │          │                 │
│ GPT-4 + Raw HTML            │  14.1%   │  42.3  │    85,432  │    81,234  │   95.1%   │   $2.47  │     67.2s       │
│ GPT-4 + AXTree              │  12.3%   │  38.7  │    45,221  │    41,892  │   92.6%   │   $1.31  │     52.1s       │
│ GPT-4 + SoM (Vision)        │  17.8%   │  35.2  │    12,445  │     8,234  │   66.2%   │   $3.54* │     71.3s       │
│                             │          │        │            │            │           │          │                 │
│ Oryn Results                │          │        │            │            │           │          │                 │
│ ─────────────────────────── │          │        │            │            │           │          │                 │
│ GPT-4 + Oryn (react)        │  23.4%   │  18.3  │    12,847  │     1,847  │   14.4%   │   $0.89  │     47.2s       │
│ GPT-4 + Oryn (plan-act)     │  21.1%   │  22.1  │    14,234  │     2,012  │   14.1%   │   $0.98  │     51.4s       │
│ GPT-4 + Oryn (reflexion)    │  26.2%   │  19.7  │    15,891  │     1,923  │   12.1%   │   $1.12  │     53.7s       │
│                             │          │        │            │            │           │          │                 │
│ Claude + Oryn (react)       │  25.8%   │  16.9  │    11,234  │     1,721  │   15.3%   │   $0.67  │     41.3s       │
│ Gemini + Oryn (react)       │  19.7%   │  21.4  │    13,456  │     1,934  │   14.4%   │   $0.12  │     38.7s       │
│                             │          │        │            │            │           │          │                 │
│ Prompt Ablations            │          │        │            │            │           │          │                 │
│ ─────────────────────────── │          │        │            │            │           │          │                 │
│ GPT-4 + Oryn (minimal)      │  18.9%   │  23.4  │     8,234  │     1,847  │   22.4%   │   $0.58  │     42.1s       │
│ GPT-4 + Oryn (few-shot)     │  24.7%   │  17.8  │    18,234  │     1,847  │   10.1%   │   $1.23  │     52.3s       │
│                             │          │        │            │            │           │          │                 │
└─────────────────────────────┴──────────┴────────┴────────────┴────────────┴───────────┴──────────┴─────────────────┘

* SoM cost includes vision model inference ($0.01/image × ~250 images/task)

Key Findings:
• Oryn reduces observation tokens by 85-98% compared to HTML/AXTree
• This translates to 60-75% reduction in total input tokens
• Success rate improves 40-86% with semantic observations
• Cost per task drops 64-96% compared to baselines
```

---

## 11. Implementation Roadmap

### Phase 1: Core Framework (Weeks 1-2)

**Deliverables:**
- [ ] OrynInterface implementation
- [ ] LLMProvider interface + OpenAI/Anthropic implementations
- [ ] Agent base class + ReAct agent
- [ ] PromptTemplate system
- [ ] MetricsCollector
- [ ] Basic CLI (run, inspect)

**Validation:**
- Run simple tasks end-to-end
- Verify metrics collection works

### Phase 2: Benchmarks (Weeks 3-4)

**Deliverables:**
- [ ] Benchmark interface
- [ ] MiniWoB++ loader (simplest, good for testing)
- [ ] WebShop loader
- [ ] WebArena loader
- [ ] Evaluation logic per benchmark

**Validation:**
- Run subset of each benchmark
- Verify evaluation accuracy

### Phase 3: Agents & Prompts (Weeks 5-6)

**Deliverables:**
- [ ] Plan-and-Act agent
- [ ] Reflexion agent
- [ ] RALPH agent + trajectory store
- [ ] Additional prompt templates
- [ ] Few-shot example management

**Validation:**
- Compare agent architectures on MiniWoB++
- Verify prompt templates work across agents

### Phase 4: Framework Adapters (Weeks 7-8)

**Deliverables:**
- [ ] Google ADK adapter
- [ ] OpenAI Swarm adapter
- [ ] Framework-agnostic tool definitions

**Validation:**
- Run same task through native and framework agents
- Verify metrics capture works through adapters

### Phase 5: Reporting & CLI (Weeks 9-10)

**Deliverables:**
- [ ] Full CLI (compare, matrix, download)
- [ ] Rich console output
- [ ] Comparison reports
- [ ] Export formats (JSON, CSV)

**Validation:**
- Generate publication-ready tables
- Verify reproducibility from saved configs

### Phase 6: Documentation & Release (Weeks 11-12)

**Deliverables:**
- [ ] User guide
- [ ] API documentation
- [ ] Example notebooks
- [ ] Benchmark replication guides
- [ ] PyPI package

---

## 12. Future Directions

### 12.1 Additional Benchmarks

- **AssistantBench**: Multi-step assistant tasks
- **WorkArena**: Enterprise software automation
- **VisualWebArena**: Vision-language web tasks (for comparison)

### 12.2 Advanced Features

- **Trajectory Visualization**: Interactive replay of agent runs
- **Error Analysis Dashboard**: Categorize and visualize failure modes
- **A/B Testing Framework**: Statistical significance testing
- **Continuous Benchmarking**: GitHub Actions integration

### 12.3 Research Extensions

- **Multi-Agent Benchmarks**: Tasks requiring agent coordination
- **Long-Horizon Tasks**: Multi-session, stateful workflows
- **Adversarial Testing**: Robustness to page changes
- **Human-in-the-Loop**: Benchmarks with human feedback

---

*Document Version: 1.0*
*Last Updated: January 2025*
