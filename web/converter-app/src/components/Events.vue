<script setup lang="ts">
import { friendlyProblem, friendlyFix } from '@/utils.js'
import { computed } from 'vue'

const props = defineProps({
  events: Array,
  issues: Array,
  partners: Array,
  relevant: Object,
})

const order = [
  { id: "FlagRacing", name: "Flag Racing", solo: true, },
  { id: "ChuteDogging", name: "Chute Dogging", solo: true,},
  { id: "CalfRopingOnFoot", name: "Calf Roping on Foot", solo: true,},
  { id: "SteerRiding", name: "Steer Riding", solo: true,},
  { id: "RanchSaddleBroncRiding", name: "Ranch Saddle Bronc Riding", solo: true,},
  { id: "BullRiding", name: "Bull Riding", solo: true,},
  { id: "PoleBending", name: "Pole Bending", solo: true,},

  { id: "TeamRopingHeader", name: "Team Roping Header", solo: false,},
  { id: "TeamRopingHeeler", name: "Team Roping Heeler", solo: false,},
  { id: "WildDragRace", name: "Wild Drag Race", solo: false,},
  { id: "GoatDressing", name: "Goat Dressing", solo: false,},
  { id: "SteerDeco", name: "Steer Deco", solo: false,},
]

const isSolo = Object.fromEntries(order.map((o) => [o.id, o.solo]));

const info = computed(() => {
  let { events } = props
  if (!events) { return [] }

  let info = events.reduce((acc, e) => {
      if (1 > e.round || e.round > 2) { return acc }

      const r = acc[e.rodeoEventRelId] ?? {
        "rounds": [false, false], "partners": [[], []]
      }

      r.rounds[e.round - 1] = true 
      r.partners[e.round - 1] = e.partners
      acc[e.rodeoEventRelId] = r 
      return acc
    }, {})

  return order.map((o) => { 
    return {"o": o, 
      "name": o.name, 
      "rounds": info[o.id]?.rounds ?? [false, false],
      "partners": info[o.id]?.partners ?? [[], []], 
    }
  })
})

const soloEvents = computed(() => {
  return info?.value?.filter((r) => r.o.solo) ?? [];
})

const partnerEvents = computed(() => {
  return info?.value?.filter((r) => !r.o.solo) ?? [];
})
</script>

<template>
  <div class="item-grid">
    <th>Event</th>
    <th>Round 1</th>
    <th>Round 2</th>

    <template v-for="e in info">
      <div class="item text-end">{{e.name}}</div>
      <div class="item text-center">{{e.rounds[0] ? "X" : ""}}</div>
      <div class="item text-center">{{e.rounds[1] ? "X" : ""}}</div>

      <template v-if="e.partners[0].length > 0">
        <div class="text-end">Round 1 Partner</div>
        <div>{{e.partners[0][0] ?? ""}}</div>
        <div>{{e.partners[0][1] ?? ""}}</div>
      </template>
      
      <template v-if="e.partners[1].length > 0">
        <div class="text-end">Round 1 Partner</div>
        <div>{{e.partners[1][0] ?? ""}}</div>
        <div>{{e.partners[1][1] ?? ""}}</div>
      </template>

    </template>
  </div>
</template>

<style scoped>
.item-grid {
  @apply grid grid-cols-3 justify-items-center gap-y-3;
}

.item {
  @apply border-t-2 border-indigo-500 w-full;
}

.item:nth-last-child(-n+3) {
  @apply border-b-2 border-indigo-500 w-full;
}
</style>
