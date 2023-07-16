<script setup lang="ts">
import Accordion from './Accordion.vue'
import { friendlyProblem, friendlyFix } from '@/utils.js'
import { computed, ref } from 'vue'

const props = defineProps({
  events: Array,
  issues: Array,
  partners: Array,
  relevant: Object,
})

const order = [
  { id: "BullRiding", name: "Bull Riding", solo: true,},
  { id: "RanchSaddleBroncRiding", name: "Ranch Saddle Bronc", solo: true,},
  { id: "SteerRiding", name: "Steer Riding", solo: true,},
  { id: "ChuteDogging", name: "Chute Dogging", solo: true,},
  { id: "CalfRopingOnFoot", name: "Calf Roping on Foot", solo: true,},
  { id: "MountedBreakaway", name: "Break-away Roping", solo: true,},

  { id: "BarrelRacing", name: "Barrel Racing", solo: true,},
  { id: "PoleBending", name: "Pole Bending", solo: true,},
  { id: "FlagRacing", name: "Flag Racing", solo: true, },

  { id: "TeamRopingHeader", name: "Team Roping Header", solo: false,},
  { id: "TeamRopingHeeler", name: "Team Roping Heeler", solo: false,},
  { id: "SteerDecorating", name: "Steer Deco", solo: false,},
  { id: "WildDragRace", name: "Wild Drag Race", solo: false,},
  { id: "GoatDressing", name: "Goat Dressing", solo: false,},
]

const isSolo = Object.fromEntries(order.map((o) => [o.id, o.solo]));

const info = computed(() => {
  let { events, relevant, partners, issues } = props
  if (!events || !relevant || !partners, !issues) { return [] }

  const dbPartners = partners.reduce((acc, p) => {
    if (1 > p.round || p.round > 2) { return acc }
    
    const e = acc[p.event] ?? [[], []]
    e[p.round - 1].push(relevant[p.igra_number])
    acc[p.event] = e
    return acc
  }, {})
  
  const problems = issues.reduce((acc, i) => {
    const data = i.problem.data
    if (!data.round || !data.event 
      || 1 > data.round || data.round > 2
      || data.index === undefined
      || 0 > data.index || data.index > 1
      || !i.fix.data
      ) { 
      return acc 
    }
    
    const e = acc[data.event] ?? [[[]], [[]]]
    e[data.round - 1][data.index].push(i.fix.data)
    acc[data.event] = e
    return acc
  }, {})

  const info = events.reduce((acc, e) => {
    if (1 > e.round || e.round > 2) { return acc }

    const r = acc[e.rodeoEventRelId] ?? {
      "rounds": [false, false], 
      "partners": dbPartners[e.rodeoEventRelId] ?? [[], []], 
      "regPartners": [[], []],
      "problems": problems[e.rodeoEventRelId] ?? [[], []],
    }

    r.rounds[e.round - 1] = true 
    r.regPartners[e.round - 1] = e.partners
    acc[e.rodeoEventRelId] = r 
    return acc
  }, {})

  return order.map((o) => { 
    return {
      "o": o, 
      "name": o.name, 
      "rounds": info[o.id]?.rounds ?? [false, false],
      "partners": info[o.id]?.partners ?? [[], []], 
      "regPartners": info[o.id]?.regPartners ?? [[], []], 
      "problems": info[o.id]?.problems ?? [[], []],
    }
  })
})

const soloEvents = computed(() => {
  return info?.value?.filter((r) => r.o.solo) ?? [];
})

const partnerEvents = computed(() => {
  return info?.value?.filter((r) => !r.o.solo) ?? [];
})

const showEvents = ref(false);
</script>

<template>
  <accordion class="w-[600px] min-w-full max-w-screen-lg"> 
    <template #summary>
      <div class="bg-green-300">
      {{events.length}} Go-Rounds
      </div>
    </template>

    <template #content>
      <div class="item-grid">
        <th>Event</th>
        <th>1st Go</th>
        <th>2nd Go</th>

        <template v-for="e in info">
          <div class="item text-end">{{e.name}}</div>
          <div class="item text-center">{{e.rounds[0] ? "X" : ""}}</div>
          <div class="item text-center">{{e.rounds[1] ? "X" : ""}}</div>

          <template v-if="e.regPartners[0].length > 0">
            <div class="place-self-start ms-4 col-span-3">
              <header>Go 1 Partners:</header>
              <div>
                Given: {{e.regPartners[0][0] ?? "----"}}
                <template v-if="e.regPartners[0][1]">&amp; {{e.regPartners[0][1]}}
                </template>
              </div>

              <div v-if="e.partners[0].length > 0">
                <div v-for="p in e.partners[0]">
                Found: {{p.igra_number}} {{p.first_name}} {{p.last_name}}
                </div>
              </div>
              <div v-else>
                No Match Found
              </div>

              <div v-if="e.problems[0].length > 0">
                <div v-if="e.problems[0][0]">
                  <div>No perfect match for: {{e.regPartners[0][0]}}</div>
                  <ul>
                    <li v-for="pm in e.problems[0][0]">
                      Possible Match: 
                      {{relevant[pm]?.igra_number}} |
                      {{relevant[pm]?.legal_first}}
                      {{relevant[pm]?.legal_last}}  aka
                      {{relevant[pm]?.first_name}}
                      {{relevant[pm]?.last_name}}
                    </li>
                  </ul>
                </div>
              </div>
            </div>
          </template>

          <template v-if="e.regPartners[1].length > 0">
            <div class="place-self-start ms-4 col-span-3">
              <header>Go 2 Partners:</header>
              <div>
                Given: {{e.regPartners[1][0] ?? "----"}}
                <template v-if="e.regPartners[1][1]">&amp; {{e.regPartners[1][1]}}
                </template>
              </div>
              
              <div v-if="e.partners[1]">
                <div v-for="p in e.partners[1]">
                Found: {{p.igra_number}} {{p.first_name}} {{p.last_name}}
                </div>
              </div>
              <div v-else>
                No Match Found
              </div>

              <div v-if="e.problems[1].length > 0">
                <div v-if="e.problems[1][0]">
                  <div>No perfect match for: {{e.regPartners[1][0]}}</div>
                  <ul>
                    <li v-for="pm in e.problems[1][0]">
                      Possible Match: 
                      {{relevant[pm]?.igra_number}} |
                      {{relevant[pm]?.legal_first}}
                      {{relevant[pm]?.legal_last}}  aka
                      {{relevant[pm]?.first_name}}
                      {{relevant[pm]?.last_name}}
                    </li>
                  </ul>
                </div>
              </div>

            </div>
            
          </template>

        </template>
      </div>
    </template>
  </accordion>
</template>

<style scoped>
.item-grid {
  @apply grid grid-cols-3 justify-items-center gap-y-3;
  @apply border-t-2 border-b-2 border-indigo-500 w-full;
}

.item {
  @apply border-t-2 border-indigo-500 w-full;
}
</style>

